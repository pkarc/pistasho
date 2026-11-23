#!/usr/bin/env python3
"""Training data for the local decision model, made from the tldr pages themselves.

Each tldr example is a (what it does, the command) pair: the English description is a question
that the command answers, and when the page has a Spanish translation its Spanish description is
a Spanish one. The wrong answers are what `pistasho ask` really retrieves for that text, so the
model learns to tell the right command from the lookalikes it will actually be shown, and a share
of items has the right tool taken out so that "none of these" is learnable too.

No third party is involved: nothing here calls an API, and nothing comes from your history, cheats
or the eval banks. The texts are tldr's (CC BY 4.0, attribute them if a model is published).

Known limit: tldr descriptions read like documentation, not like what a person types ("Create
directories and set their permissions" vs "crear una carpeta nueva"). The English side is
augmented (word dropout, "how to ...") to break the verbatim match with the indexed text; whether it
generalises is what the eval banks measure.

    XDG_DATA_HOME=... XDG_CONFIG_HOME=... python3 training/make_data.py --out training/out/data [--limit 300]
"""
import argparse
import hashlib
import json
import random
import re
import sqlite3
import tomllib
import sys
from pathlib import Path

from common import FORMATS, MAX_OPTIONS, ROOT, build_row, local_candidates_batch, right
from style import casual
import os

ES_WORDS = {"el", "la", "los", "las", "de", "del", "un", "una", "para", "con", "en", "y", "se", "que", "por", "al", "su", "sus", "los"}
ES_ACCENTS = re.compile(r"[áéíóúñ¿¡]")


def clean(text):
    text = re.sub(r"`", "", text)
    text = re.sub(r"\{\{(.*?)\}\}", lambda m: re.sub(r"[\[\]|]", " ", m.group(1)).strip(), text)
    return re.sub(r"\s+", " ", text).strip().rstrip(".:").strip()


def shape(cmd):
    return re.sub(r"\{\{.*?\}\}", "{{}}", cmd)


def looks_spanish(text):
    words = set(re.findall(r"[a-záéíóúñ]+", text.lower()))
    return bool(ES_ACCENTS.search(text.lower())) or len(words & ES_WORDS) >= 2


_INVERSE = None


def inverse_synonyms(vocab):
    """English word -> Spanish words a person would say, from the synonym table pistasho already ships.

    Only the first English word of each entry is its plain translation (`carpeta = ["directorio",
    "directory", "folder"]`: the first is Spanish, the rest are what tldr says), and only words that
    tldr's English descriptions really use; the later ones are loose neighbours ("permisos" ->
    "mode") that would make wrong pairs."""
    global _INVERSE
    if _INVERSE is None:
        table = tomllib.loads((ROOT / "data/synonyms.toml").read_text())["synonyms"]
        inv = {}
        for es, ens in table.items():
            english = [w for w in ens if w in vocab and w.isascii() and w not in table]
            if english and english[0] == ens[0] or (english and ens[0] not in vocab):
                inv.setdefault(english[0], [])
                if es not in inv[english[0]]:
                    inv[english[0]].append(es)
        _INVERSE = inv
    return _INVERSE


def code_switch(text, rng, vocab):
    """The English description with the words the synonym table knows said in Spanish, like a Spanish
    speaker asking for an English-documented command ("crear directory", "borrar files"). Targets the
    Spanish -> English gap, which is where the local search fails most."""
    inv = inverse_synonyms(vocab)
    words, changed = text.split(), 0
    for i, w in enumerate(words):
        base = re.sub(r"[^a-z]", "", w.lower())
        options = inv.get(base) or inv.get(base.rstrip("s"))
        if options and rng.random() < 0.7:
            words[i] = rng.choice(options)
            changed += 1
    return " ".join(words) if changed else None


def lcfirst(s):
    return s[:1].lower() + s[1:]


def variants(desc_en, alt, rng, mix=0.0, vocab=frozenset()):
    """The questions a tldr example can stand for: (text, language)."""
    out = []
    en = clean(desc_en)
    if en:
        r = rng.random()
        words = en.split()
        if r < 0.35:
            out.append((lcfirst(en), "en"))
        elif r < 0.65:
            out.append((rng.choice(["how to ", "how do I ", "i want to ", "command to "]) + lcfirst(en), "en"))
        else:
            kept = [w for i, w in enumerate(words) if i == 0 or rng.random() > 0.25]
            out.append((lcfirst(" ".join(kept if len(kept) >= 3 else words)), "en"))
    if en and rng.random() < mix:
        switched = code_switch(lcfirst(en), rng, vocab)
        if switched:
            out.append((rng.choice(["", "cómo ", "quiero ", "necesito "]) + switched, "mix"))
    first = alt.split(" · ")[0] if " · " in alt else ""
    es = clean(first)
    if es and looks_spanish(es) and es.lower() != en.lower():
        r = rng.random()
        if r < 0.5:
            out.append((lcfirst(es), "es"))
        else:
            out.append((rng.choice(["cómo ", "quiero ", "necesito ", "comando para "]) + lcfirst(es), "es"))
    return out


def load_entries(db):
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute("select tool, description, alt, cmd from entries where source='tldr' order by id").fetchall()
    con.close()
    return [{"tool": t, "desc": d, "alt": a, "cmd": c} for t, d, a, c in rows]


def retrieve(queries, cache_path, workers):
    """Candidates for every query, through `pistasho ask`, remembered in a JSONL cache (resumable)."""
    cache = {}
    if cache_path.exists():
        for line in cache_path.read_text().splitlines():
            rec = json.loads(line)
            cache[rec["q"]] = rec["c"]
    todo = [q for q in dict.fromkeys(queries) if q not in cache]
    print(f"{len(cache)} cached, {len(todo)} to retrieve with {workers} workers", file=sys.stderr)
    # in slices, so an interrupted run keeps what it did and the cache file grows as it goes
    step = 4000
    with cache_path.open("a") as f:
        for start in range(0, len(todo), step):
            part = todo[start : start + step]
            found = local_candidates_batch(part, MAX_OPTIONS, workers)
            for q in part:
                cands = found.get(" ".join(q.split()), [])
                cache[q] = cands
                f.write(json.dumps({"q": q, "c": cands}, ensure_ascii=False) + "\n")
            f.flush()
            print(f"  {min(start + step, len(todo))}/{len(todo)}", file=sys.stderr)
    return cache


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", default=None, help="not needed: read from the XDG data dir like pistasho does")
    ap.add_argument("--out", required=True)
    ap.add_argument("--limit", type=int, default=0, help="only this many tldr entries (for a smoke test)")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--none-share", type=float, default=0.10)
    ap.add_argument("--cands", type=int, default=MAX_OPTIONS, help="options per decision (the rest of the retrieved ones are dropped)")
    ap.add_argument("--fmt", choices=FORMATS, default="td", help="how an option reads (common.option_text)")
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--generated", nargs="*", default=[], help="JSONL from gen_questions.py: human-sounding questions per example")
    ap.add_argument("--generated-only", action="store_true", help="use only the generated questions, not the ones derived from the descriptions")
    ap.add_argument("--casual", type=float, default=0.8, help="share of the questions written the way people type (lowercase, no accents, no ¿ ¡ or final stop); see style.py")
    ap.add_argument("--toollevel", nargs="*", default=[], help="JSONL from import_nl2bash.py: questions whose answer is a tool, not one example")
    ap.add_argument("--max-per-tool", type=int, default=0, help="at most this many questions per tool (0 = no limit): tldr has hundreds of examples of cloud CLIs and few of ls, tar or grep")
    ap.add_argument("--augment", type=float, default=0.0, help="share of questions rewritten as a short query (2-3 content words) or with a typo, half each; own random stream, so 0 changes nothing")
    ap.add_argument("--mix", type=float, default=0.25, help="share of entries that also get a code-switched question (0 = none)")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    db = args.db or str(Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "pistasho/pistasho.db")
    entries = load_entries(db)
    # every example of every tool, whatever --limit keeps: the tool-level questions look tools up here
    by_tool = {}
    for e in entries:
        by_tool.setdefault(e["tool"], []).append(e)
    if args.limit:
        rng.shuffle(entries)
        entries = entries[: args.limit]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    vocab = {w for e in entries for w in re.findall(r"[a-z]+", e["desc"].lower())}
    generated = {}
    for path in args.generated:
        for line in open(path):
            rec = json.loads(line)
            generated.setdefault((rec["tool"], rec["cmd"]), []).extend((t, l, "gen-" + Path(path).stem) for l, t in rec["questions"])
    aug_rng = random.Random(args.seed + 1)

    def augment(text):
        """What people also type: two or three content words, or a slip of the fingers. Never fewer than two words."""
        if args.augment <= 0:
            return text
        r = aug_rng.random()
        if r >= args.augment:
            return text
        words = text.split()
        if r < args.augment / 2:
            stop = set("como cómo how to do i a an the el la los las un una de del para que por con en and y o or want quiero necesito me my mi mis comando command can puedo se is it that this what cual cuál es are of on in".split())
            content = [w for w in words if w.lower() not in stop]
            return " ".join(content[: aug_rng.choice((2, 3))]) if len(content) >= 2 else text
        long_ = [i for i, w in enumerate(words) if len(w) >= 5 and w.isalpha()]
        if not long_:
            return text
        i = aug_rng.choice(long_)
        w = words[i]
        k = aug_rng.randrange(1, len(w) - 1)
        op = aug_rng.choice(("swap", "drop", "double"))
        words[i] = w[:k] + w[k + 1] + w[k] + w[k + 2:] if op == "swap" else w[:k] + w[k + 1:] if op == "drop" else w[:k] + w[k] + w[k:]
        return " ".join(words)

    work = []
    for e in entries:
        derived = [] if args.generated_only else variants(e["desc"], e["alt"], rng, args.mix, vocab)
        # the source tag only labels the row (meta files); it does not touch the random draws
        for text, lang, src in [(t, l, "tldr-" + l) for t, l in derived] + generated.get((e["tool"], e["cmd"]), []):
            if len(text.split()) >= 2:
                work.append((e, augment(casual(text, rng, args.casual)), lang, src))
    # tool-level questions (NL2Bash): the answer is any example of the tool(s) the one-liner runs
    tool_work = []
    for path in args.toollevel:
        for line in open(path):
            rec = json.loads(line)
            tool_work.append((rec["tools"], casual(rec["question"], rng, args.casual)))
    if args.max_per_tool:
        # a random cut per tool, before the (slow) retrieval; the same seed gives the same cut
        def cap(items, tool_of):
            rng.shuffle(items)
            seen, kept = {}, []
            for it in items:
                t = tool_of(it)
                seen[t] = seen.get(t, 0) + 1
                if seen[t] <= args.max_per_tool:
                    kept.append(it)
            return kept

        work = cap(work, lambda w: w[0]["tool"])
        tool_work = cap(tool_work, lambda w: w[0][0])
    print(f"{len(entries)} entries -> {len(work)} questions, {len(tool_work)} tool-level", file=sys.stderr)
    cache = retrieve([w[1] for w in work] + [t[1] for t in tool_work], out / "candidates.jsonl", args.workers)

    stats = {"train": 0, "valid": 0, "none": 0, "injected": 0, "skipped_no_cands": 0, "skipped_short": 0}
    # one line per row, same order as train.jsonl / valid.jsonl: where the question came from and whether the
    # local order (before shuffling, injecting or removing anything) had a right tool in first place
    with (out / "train.jsonl").open("w") as ftrain, (out / "valid.jsonl").open("w") as fvalid, \
            (out / "train.meta.jsonl").open("w") as mtrain, (out / "valid.meta.jsonl").open("w") as mvalid:
        def meta(held, **kw):
            (mvalid if held else mtrain).write(json.dumps(kw, ensure_ascii=False) + "\n")

        for e, text, lang, src in work:
            cands = [dict(c, rank=r + 1) for r, c in enumerate(cache.get(text) or [])]   # rank = place in the local order (used by --fmt tdr)
            if len(cands) < 5:
                stats["skipped_no_cands"] += 1
                continue
            mine = shape(e["cmd"])
            loc_tool = right(cands[0]["tool"], [e["tool"]])
            loc_exact = cands[0]["tool"] == e["tool"] and shape(cands[0]["command"]) == mine
            pos = [i for i, c in enumerate(cands) if c["tool"] == e["tool"] and shape(c["command"]) == mine]
            had_exact = bool(pos)   # the exact example was among the retrieved candidates (else it is injected below)
            make_none = rng.random() < args.none_share
            if make_none:
                # take the right tool out altogether: what is left cannot be the answer
                cands = [c for c in cands if c["tool"] != e["tool"]]
                if len(cands) < 5:
                    stats["skipped_short"] += 1
                    continue
                label = "none"
            else:
                if not pos:
                    # the right command was not retrieved: put it in (the model cannot learn from a miss)
                    cands = cands[: args.cands - 1] + [{"tool": e["tool"], "description": e["desc"], "command": e["cmd"]}]
                    pos = [len(cands) - 1]
                    stats["injected"] += 1
                label = None
            rng.shuffle(cands)
            if label is None:
                # the exact example, and its siblings of the same tool as a lesser right answer
                exact = next(i for i, c in enumerate(cands) if c["tool"] == e["tool"] and shape(c["command"]) == mine)
                sib = [i for i, c in enumerate(cands) if c["tool"] == e["tool"] and i != exact]
                probs = {f"c{i + 1}": 0.0 for i in range(len(cands))}
                probs["none"] = 0.0
                if sib:
                    probs[f"c{exact + 1}"] = 0.7
                    for i in sib:
                        probs[f"c{i + 1}"] = 0.3 / len(sib)
                else:
                    probs[f"c{exact + 1}"] = 1.0
            else:
                probs = {f"c{i + 1}": 0.0 for i in range(len(cands))}
                probs["none"] = 1.0
                stats["none"] += 1
            row = build_row(text, cands[: args.cands], args.fmt)
            row["gold"] = {"pick": {"probabilities": probs}}
            # validation holds out whole tools, so it measures tools the model never saw
            held_out = int(hashlib.md5(e["tool"].encode()).hexdigest(), 16) % 20 == 0
            (fvalid if held_out else ftrain).write(json.dumps(row, ensure_ascii=False) + "\n")
            meta(held_out, src=src, q=text, none=label == "none", loc_tool=loc_tool, loc_exact=loc_exact, had=had_exact)
            stats["valid" if held_out else "train"] += 1
        # tool-level items: every tool of the one-liner is a right answer, equally; "none" takes every one of
        # them out
        for tools, text in tool_work:
            cands = [dict(c, rank=r + 1) for r, c in enumerate(cache.get(text) or [])]
            primary = tools[0]
            loc_tool = bool(cands) and right(cands[0]["tool"], tools)
            had_tool = any(c["tool"] == primary for c in cands)
            if len(cands) < 5 or primary not in by_tool:
                stats["skipped_no_cands"] += 1
                continue
            if rng.random() < args.none_share:
                cands = [c for c in cands if c["tool"] not in tools]
                if len(cands) < 5:
                    stats["skipped_short"] += 1
                    continue
                label = "none"
            else:
                if not any(c["tool"] == primary for c in cands):
                    e = rng.choice(by_tool[primary])
                    cands = cands[: args.cands - 1] + [{"tool": e["tool"], "description": e["desc"], "command": e["cmd"]}]
                    stats["injected"] += 1
                label = None
            rng.shuffle(cands)
            cands = cands[: args.cands]
            probs = {f"c{i + 1}": 0.0 for i in range(len(cands))}
            probs["none"] = 0.0
            if label == "none":
                probs["none"] = 1.0
                stats["none"] += 1
            else:
                # a one-liner `find . | xargs grep x` is answered as well by `grep` as by `find`: each tool it
                # runs weighs the same, split among that tool's examples
                present = {t: [i for i, c in enumerate(cands) if c["tool"] == t] for t in tools}
                present = {t: ix for t, ix in present.items() if ix}
                if not present:
                    stats["skipped_short"] += 1
                    continue
                for ix in present.values():
                    for i in ix:
                        probs[f"c{i + 1}"] = 1.0 / len(present) / len(ix)
            row = build_row(text, cands, args.fmt)
            row["gold"] = {"pick": {"probabilities": probs}}
            held_out = int(hashlib.md5(primary.encode()).hexdigest(), 16) % 20 == 0
            (fvalid if held_out else ftrain).write(json.dumps(row, ensure_ascii=False) + "\n")
            meta(held_out, src="nl2bash", q=text, none=label == "none", loc_tool=loc_tool, loc_exact=None, had=had_tool)
            stats["valid" if held_out else "train"] += 1
    print(json.dumps(stats), file=sys.stderr)


if __name__ == "__main__":
    main()
