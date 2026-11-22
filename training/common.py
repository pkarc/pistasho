"""What the decision model is asked, shared by every script here.

It must stay identical to what pistasho sends at run time (src/decide.rs `request_choice`): a
model trained on one wording and asked another would be measured on a different task.
"""
import json
import os
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STATE = "User request: {question}"
INSTRUCTIONS = "Which command does what the user asked for?"
NONE_TEXT = "None of these commands does what the user asked for."
MAX_OPTIONS = 20  # decide::MAX_OPTIONS


FORMATS = ("td", "tdc")


def option_text(c, fmt="td"):
    """One candidate as the model reads it. `tdc` is `tool: description -> command`, what the hosted
    model is shown; `td` drops the command. Laya reads all the options inside one token budget
    (`head_max_len`, 256 in the published checkpoint: about 11 tokens each for 20 options), so the
    command mostly pushes the description out; which one works better is an experiment."""
    if fmt == "tdc":
        return f'{c["tool"]}: {c["description"]} -> {c["command"]}'
    return f'{c["tool"]}: {c["description"]}'


def build_row(question, cands, fmt="td", n=None):
    """The `{state, questions}` of one decision: the first `n` candidates as options `c1`..`cN`, then `none`."""
    cands = cands[:n] if n else cands
    criteria = {f"c{i + 1}": option_text(c, fmt) for i, c in enumerate(cands)}
    criteria["none"] = NONE_TEXT
    return {
        "state": STATE.format(question=question),
        "questions": {"pick": {"type": "choice", "instructions": INSTRUCTIONS, "criteria": criteria}},
    }


def right(tool, wanted):
    """A tool answers a question if it is a wanted one or one of its subcommand pages."""
    return any(tool == w or tool.startswith(w + "-") for w in wanted)


def local_candidates(question, n=MAX_OPTIONS, binary=None, env=None):
    """The best local candidates for a question, exactly as `pistasho ask` ranks them."""
    binary = binary or os.environ.get("PISTASHO_BIN", str(ROOT / "target/release/pistasho"))
    out = subprocess.run(
        [binary, "ask", "--json", "-n", str(n), question],
        capture_output=True, text=True, env=env or dict(os.environ, HISTFILE="/dev/null"),
    )
    if not out.stdout.strip():
        return []
    # only public tldr text ever reaches a model; with a clean database that is everything
    return [c for c in json.loads(out.stdout)["candidates"] if c["source"] == "tldr"]


def local_candidates_batch(questions, n=MAX_OPTIONS, workers=4, binary=None, env=None):
    """`local_candidates` for many questions at once: `pistasho ask --batch` loads the index and the
    model once per process, so tens of thousands of questions take minutes instead of hours.
    Returns {question: [candidates]}."""
    binary = binary or os.environ.get("PISTASHO_BIN", str(ROOT / "target/release/pistasho"))
    env = env or dict(os.environ, HISTFILE="/dev/null")
    questions = [" ".join(q.split()) for q in questions]
    chunks = [questions[i::workers] for i in range(workers)]

    def run(chunk):
        if not chunk:
            return {}
        out = subprocess.run([binary, "ask", "--batch", "-n", str(n)], input="\n".join(chunk) + "\n",
                             capture_output=True, text=True, env=env)
        got = {}
        for line in out.stdout.splitlines():
            rec = json.loads(line)
            got[rec["question"]] = [c for c in rec["candidates"] if c["source"] == "tldr"]
        return got

    from concurrent.futures import ThreadPoolExecutor

    result = {}
    with ThreadPoolExecutor(workers) as pool:
        for part in pool.map(run, chunks):
            result.update(part)
    return result


def eval_questions():
    """The two eval banks: `orig` (real) and `synth` (generated); never trained on."""
    banks = {}
    for name, path in (("orig", ROOT / "eval/questions.toml"), ("synth", ROOT / "eval/questions-synth.toml")):
        banks[name] = tomllib.loads(path.read_text())["q"]
    return banks
