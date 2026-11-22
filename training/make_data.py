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

from common import FORMATS, MAX_OPTIONS, ROOT, build_row, local_candidates_batch
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
    ap.add_argument("--mix", type=float, default=0.25, help="share of entries that also get a code-switched question (0 = none)")
    args = ap.parse_args()

    rng = random.Random(args.seed)
    db = args.db or str(Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "pistasho/pistasho.db")
    entries = load_entries(db)
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
            generated.setdefault((rec["tool"], rec["cmd"]), []).extend((t, l) for l, t in rec["questions"])
    work = []
    for e in entries:
        derived = [] if args.generated_only else variants(e["desc"], e["alt"], rng, args.mix, vocab)
        for text, lang in derived + generated.get((e["tool"], e["cmd"]), []):
            if len(text.split()) >= 2:
                work.append((e, text, lang))
    print(f"{len(entries)} entries -> {len(work)} questions", file=sys.stderr)
    cache = retrieve([w[1] for w in work], out / "candidates.jsonl", args.workers)

    stats = {"train": 0, "valid": 0, "none": 0, "injected": 0, "skipped_no_cands": 0, "skipped_short": 0}
    with (out / "train.jsonl").open("w") as ftrain, (out / "valid.jsonl").open("w") as fvalid:
        for e, text, lang in work:
            cands = list(cache.get(text) or [])
            if len(cands) < 5:
                stats["skipped_no_cands"] += 1
                continue
            mine = shape(e["cmd"])
            pos = [i for i, c in enumerate(cands) if c["tool"] == e["tool"] and shape(c["command"]) == mine]
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
            stats["valid" if held_out else "train"] += 1
    print(json.dumps(stats), file=sys.stderr)


if __name__ == "__main__":
    main()
