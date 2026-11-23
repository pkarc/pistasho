#!/usr/bin/env python3
"""Human-sounding questions for the tldr examples, written by an OPEN-WEIGHTS model on a hosted service.

tldr describes what a command does like documentation ("Create directories and set their
permissions"); people type "crear una carpeta". For each example this asks a model to write a few
questions the way a person who does not know the command would ask, in Spanish and English. The
label comes for free: the question was written FROM that example.

Which model and which service matter, because of their terms: never Anthropic/Claude or Jev (their
terms forbid training on their outputs, see docs/training.md). Use a model whose license allows it
(Apache-2.0: Qwen3.5, Gemma 4) on a service whose terms do not restrict it (read for DeepInfra,
Together AI and OpenRouter on 2026-11-22). Only public tldr text goes out: the tool name, the
description and the command template; nothing from your history, cheats or files.

Any OpenAI-compatible endpoint works. The key is read from an environment variable, never printed.

    export DEEPINFRA_API_KEY=...        # or OPENROUTER_API_KEY, TOGETHER_API_KEY
    python3 training/gen_questions.py --base-url https://api.deepinfra.com/v1/openai \\
        --model google/gemma-4-31B-it --key-env DEEPINFRA_API_KEY --limit 200 --out training/out/gen/gemma.jsonl
    # Qwen3.5 thinks by default; switch it off (the field depends on the service):
    #   --extra-json '{"chat_template_kwargs": {"enable_thinking": false}}'

Resumable: examples already in --out are skipped.
"""
import argparse
import json
import os
import random
import re
import sqlite3
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

PROMPT = """You write the questions real people type when they need a shell command and do not know \
which one. For each numbered command below, write three questions a person could ask that this \
specific command answers: two in Spanish and one in English (or none, see the last rule).

Rules:
{style}
- Never name the command, its tool or its flags, and do not copy the description: the person does \
not know them.
- Specific enough that this command, and not a generic one of the same tool, is the answer (keep \
the details the description gives: a file type, an option's effect, a direction).
- No placeholders, no quotes around the question.
- If the description is too generic for any natural question to point at THIS command rather than the \
same option of any other tool ("display help", "enable verbose output", "show the version"), return \
an empty list of questions for it: a wrong label is worse than no example.

Answer with JSON only, in exactly this shape and nothing else:
{{"items": [{{"id": 1, "questions": [{{"lang": "es", "text": "..."}}, {{"lang": "es", "text": "..."}}, {{"lang": "en", "text": "..."}}]}}]}}

Commands:
{items}"""

# The first pass asks for a direct request, a "how do I", and a goal. A second pass with another mix of
# styles gives more variety out of the same examples.
STYLES = {
    "A": ('- Sound like a person, not like documentation: casual, short (3 to 14 words), different from each '
          'other (one direct request, one phrased as "cómo hago para..." / "how do I...", one that states the '
          'goal or the problem instead of the action).'),
    "B": ("- Sound like a person, not like documentation, and write each of the three in a different style: one "
          "very short, like a search query (2 to 5 words); one that tells the situation or the problem without "
          "naming the action; one a beginner would write, with everyday words instead of technical ones."),
}
PROMPTS = {k: PROMPT.replace("{style}", v) for k, v in STYLES.items()}
LOCK = threading.Lock()


def load_entries(db):
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute("select tool, description, cmd from entries where source='tldr' order by id").fetchall()
    con.close()
    return [{"tool": t, "desc": d, "cmd": c} for t, d, c in rows]


def render(group):
    return "\n".join(f'{n}. {e["desc"].strip()}\n   command: {e["cmd"]}' for n, e in enumerate(group, 1))


def extract_json(text):
    """The JSON object in a model's answer, which may come in a code fence or with a lead-in."""
    text = re.sub(r"<think>.*?</think>", "", text, flags=re.S).strip()
    text = re.sub(r"^```(?:json)?\s*|\s*```$", "", text)
    start, end = text.find("{"), text.rfind("}")
    if start < 0 or end <= start:
        raise ValueError("no JSON object")
    return json.loads(text[start : end + 1])


def clean_questions(entry, questions):
    """Keeps the questions a person could plausibly type: the right length, not naming the tool or its flags."""
    tool_words = {w for w in re.split(r"[-_\s]+", entry["tool"].lower()) if len(w) >= 3}
    out = []
    for q in questions:
        lang, text = q.get("lang"), " ".join(str(q.get("text", "")).split()).strip("\"'“”")
        words = text.split()
        if lang not in ("es", "en") or not 2 <= len(words) <= 20:
            continue
        low = text.lower()
        if "--" in text or re.search(r"(^|\s)-[a-zA-Z]\b", text) or "{{" in text:
            continue
        if any(re.search(rf"\b{re.escape(w)}\b", low) for w in tool_words):
            continue
        if low not in {x[1].lower() for x in out}:
            out.append((lang, text))
    return out


def call(args, key, group):
    body = {
        "model": args.model,
        "messages": [{"role": "user", "content": PROMPTS[args.variant].format(items=render(group))}],
        "temperature": args.temperature,
        "max_tokens": 3000,
    }
    body.update(json.loads(args.extra_json))
    data = json.dumps(body).encode()
    for attempt in range(5):
        req = urllib.request.Request(
            args.base_url.rstrip("/") + "/chat/completions", data=data,
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
        )
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                out = json.load(r)
            choice = out["choices"][0]
            usage = out.get("usage") or {}
            return choice["message"]["content"] or "", usage.get("prompt_tokens", 0), usage.get("completion_tokens", 0)
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503, 504) and attempt < 4:
                time.sleep(2 ** attempt * 2)
                continue
            # never echo the request; the service's own message is enough
            raise RuntimeError(f"HTTP {e.code}: {e.read()[:200].decode(errors='replace')}")
        except (urllib.error.URLError, TimeoutError) as e:
            if attempt < 4:
                time.sleep(2 ** attempt * 2)
                continue
            raise RuntimeError(f"network: {e}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--key-env", required=True, help="name of the environment variable that holds the API key")
    ap.add_argument("--db", default=str(Path.home() / ".local/share/pistasho/pistasho.db"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--limit", type=int, default=0, help="only this many tldr examples (random, fixed seed)")
    ap.add_argument("--per-call", type=int, default=8)
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--temperature", type=float, default=0.8)
    ap.add_argument("--seed", type=int, default=11)
    ap.add_argument("--variant", choices=["A", "B"], default="A", help="prompt variant (B = another mix of styles)")
    ap.add_argument("--price-in", type=float, default=0.0, help="USD per million input tokens, for --budget")
    ap.add_argument("--price-out", type=float, default=0.0, help="USD per million output tokens, for --budget")
    ap.add_argument("--budget", type=float, default=0.0, help="stop sending calls once this many USD were spent (0 = no limit)")
    ap.add_argument("--extra-json", default="{}", help="extra fields for the request body, e.g. to switch thinking off")
    args = ap.parse_args()

    key = os.environ.get(args.key_env, "").strip()
    if not key:
        sys.exit(f"set {args.key_env}")
    entries = load_entries(args.db)
    rng = random.Random(args.seed)
    rng.shuffle(entries)
    if args.limit:
        entries = entries[: args.limit]
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    done = set()
    if out.exists():
        for line in out.read_text().splitlines():
            rec = json.loads(line)
            done.add((rec["tool"], rec["cmd"]))
    todo = [e for e in entries if (e["tool"], e["cmd"]) not in done]
    groups = [todo[i : i + args.per_call] for i in range(0, len(todo), args.per_call)]
    print(f"{len(done)} done, {len(todo)} to do in {len(groups)} calls ({args.model})", file=sys.stderr)
    stats = {"in": 0, "out": 0, "bad_json": 0, "failed": 0, "written": 0, "empty": 0, "dropped": 0}

    def work(group):
        if args.budget and (stats["in"] * args.price_in + stats["out"] * args.price_out) / 1e6 >= args.budget:
            with LOCK:
                stats["skipped_budget"] = stats.get("skipped_budget", 0) + 1
            return
        try:
            text, tin, tout = call(args, key, group)
            items = extract_json(text)["items"]
        except (ValueError, KeyError, TypeError, RuntimeError) as e:
            with LOCK:
                stats["failed" if isinstance(e, RuntimeError) else "bad_json"] += 1
                # the message only: it never contains the request or the key
                print(f"  ! {type(e).__name__}: {str(e)[:160]}", file=sys.stderr)
            return
        lines = []
        for it in items:
            n = it.get("id")
            if not (isinstance(n, int) and 1 <= n <= len(group)):
                continue
            e = group[n - 1]
            raw = it.get("questions") or []
            kept = clean_questions(e, raw)
            with LOCK:
                stats["dropped"] += len(raw) - len(kept)
                stats["empty"] += not kept
            lines.append(json.dumps({"tool": e["tool"], "cmd": e["cmd"], "desc": e["desc"], "questions": kept}, ensure_ascii=False))
        with LOCK:
            stats["in"] += tin
            stats["out"] += tout
            stats["written"] += len(lines)
            with out.open("a") as f:
                f.write("\n".join(lines) + ("\n" if lines else ""))

    with ThreadPoolExecutor(args.workers) as pool:
        for i, _ in enumerate(pool.map(work, groups), 1):
            if i % 50 == 0:
                print(f"  {i}/{len(groups)} calls, {stats['in']} in / {stats['out']} out tokens", file=sys.stderr)
    print(json.dumps(stats), file=sys.stderr)


if __name__ == "__main__":
    main()
