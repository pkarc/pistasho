#!/usr/bin/env python3
"""Would Jev (TypeSafe's hosted decision model) pay off? Measured before building a client (task 3.x).

Two experiments against POST https://api.typesafe.ai/v1/systemone, key from TYPESAFE_API_KEY (never
printed). Only eval questions and tldr candidates are sent: no history, no cheats, no private data.

  choice   for every eval question: the 10 best local candidates + "none" as a `choice`; does it
           beat the local top-1? does it know when none is right? (what the LLM trigger could use)
  risk     a labelled list of commands as a `noul` "is this destructive?" against the fixed rules
           (`pistasho risk`), to see whether a model would add anything to them

Needs a database built by `pistasho update` (XDG_DATA_HOME / XDG_CONFIG_HOME) and the release
binary. Results are cached in ~/.cache/pk-e so a rerun costs nothing.

    XDG_DATA_HOME=... python3 scripts/jev_experiment.py choice
    python3 scripts/jev_experiment.py risk
"""
import json
import os
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = os.environ.get("PISTASHO_BIN", str(ROOT / "target/release/pistasho"))
CACHE = Path(os.environ.get("JEV_CACHE", Path.home() / ".cache/pk-e"))
URL = os.environ.get("TYPESAFE_BASE_URL", "https://api.typesafe.ai").rstrip("/") + "/v1/systemone"
SETS = {"orig": ROOT / "eval/questions.toml", "synth": ROOT / "eval/questions-synth.toml"}
CANDS = int(os.environ.get("JEV_CANDS", "10"))
WORKERS = 4


def call(state, questions, retries=4):
    key = os.environ.get("TYPESAFE_API_KEY")
    if not key:
        sys.exit("set TYPESAFE_API_KEY")
    body = json.dumps({"model": "jev-latest", "state": state, "questions": questions}).encode()
    for attempt in range(retries):
        req = urllib.request.Request(
            URL, data=body, headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"}
        )
        t0 = time.time()
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                out = json.load(r)
            out["_secs"] = time.time() - t0
            return out
        except urllib.error.HTTPError as e:
            if e.code in (429, 500, 502, 503) and attempt < retries - 1:
                time.sleep(2 ** attempt)
                continue
            return {"error": f"HTTP {e.code}", "_secs": time.time() - t0}
        except Exception as e:  # network trouble must not stop the whole run
            if attempt < retries - 1:
                time.sleep(2 ** attempt)
                continue
            return {"error": str(e)[:80], "_secs": time.time() - t0}


def cached(path, items, fn):
    """Runs fn(item) for the items not in the cache file yet, WORKERS at a time."""
    data = json.loads(path.read_text()) if path.exists() else {}
    todo = [i for i in items if i[0] not in data]
    with ThreadPoolExecutor(WORKERS) as pool:
        for (k, _), res in zip(todo, pool.map(lambda i: fn(i[1]), todo)):
            if "error" not in res:
                data[k] = res
            else:
                print(f"  ! {k[:50]}: {res['error']}", file=sys.stderr)
    path.write_text(json.dumps(data))
    return data


# ---------------------------------------------------------------- choice

def local(question):
    out = subprocess.run(
        [BIN, "ask", "--json", "-n", str(CANDS), question],
        capture_output=True, text=True, env=dict(os.environ, HISTFILE="/dev/null"),
    )
    return json.loads(out.stdout)["candidates"] if out.stdout.strip() else []


def ask_choice(item):
    question, cands = item
    criteria = {
        f"c{i + 1}": f'{c["tool"]}: {c["description"]} -> {c["command"]}' for i, c in enumerate(cands)
    }
    criteria["none"] = "None of these commands does what the user asked for."
    return call(
        f"User request: {question}",
        {"pick": {"type": "choice", "instructions": "Which command does what the user asked for?", "criteria": criteria}},
    )


def right(c, wanted):
    t = c["tool"]
    return any(t == w or t.startswith(w + "-") for w in wanted)


def choice_experiment():
    for name, path in SETS.items():
        qs = tomllib.loads(path.read_text())["q"]
        cands = {q["question"]: local(q["question"]) for q in qs}
        items = [(q["question"], (q["question"], cands[q["question"]])) for q in qs if cands[q["question"]]]
        answers = cached(CACHE / f"jev-choice-{name}-{CANDS}.json", items, ask_choice)
        n = len(qs)
        rows = []
        for q in qs:
            cs = cands[q["question"]]
            a = answers.get(q["question"])
            if not cs or not a:
                rows.append(None)
                continue
            pick = a["answers"]["pick"]
            probs = pick["probabilities"]
            ranked = sorted(range(len(cs)), key=lambda i: -probs.get(f"c{i + 1}", 0.0))
            rows.append({
                "wanted": q["tools"], "cands": cs, "pick": pick["choice"], "conf": pick["confidence"],
                "none_p": probs.get("none", 0.0), "ranked": ranked, "secs": a["_secs"],
                "score": cs[0]["score"],
            })
        done = [r for r in rows if r]
        print(f"\n== {name}: {n} questions, {len(done)} answered by Jev")
        loc1 = sum(right(r["cands"][0], r["wanted"]) for r in done)
        jev1 = sum(right(r["cands"][r["ranked"][0]], r["wanted"]) for r in done)
        jev3 = sum(any(right(r["cands"][i], r["wanted"]) for i in r["ranked"][:3]) for r in done)
        loc3 = sum(any(right(c, r["wanted"]) for c in r["cands"][:3]) for r in done)
        rec = sum(any(right(c, r["wanted"]) for c in r["cands"]) for r in done)
        d = len(done)
        print(f"   local order : top-1 {loc1 / d * 100:3.0f}%  top-3 {loc3 / d * 100:3.0f}%   (recall@10 {rec / d * 100:.0f}%)")
        print(f"   Jev re-rank : top-1 {jev1 / d * 100:3.0f}%  top-3 {jev3 / d * 100:3.0f}%   (ordering the 10 by its probabilities)")
        secs = sorted(r["secs"] for r in done)
        print(f"   latency     : median {secs[len(secs) // 2] * 1000:.0f} ms, p95 {secs[int(len(secs) * .95)] * 1000:.0f} ms")
        # does "none" mean the local answer is wrong?
        none = [r for r in done if r["pick"] == "none"]
        some = [r for r in done if r["pick"] != "none"]
        if none:
            ok_none = sum(right(r["cands"][0], r["wanted"]) for r in none)
            print(f"   'none'      : {len(none)} questions ({len(none) / d * 100:.0f}%); local top-1 is right in {ok_none / len(none) * 100:.0f}% of them,"
                  f" vs {sum(right(r['cands'][0], r['wanted']) for r in some) / max(1, len(some)) * 100:.0f}% when Jev picks something")
        # the policy: trust Jev when it picks a candidate with enough confidence, else local
        print("   policy: Jev's pick when it names a candidate with confidence >= t, else the local top-1")
        for t in (0.0, 0.5, 0.8, 0.95):
            hits = used = 0
            for r in done:
                if r["pick"] != "none" and r["conf"] >= t:
                    idx = int(r["pick"][1:]) - 1
                    used += 1
                    hits += right(r["cands"][idx], r["wanted"])
                else:
                    hits += right(r["cands"][0], r["wanted"])
            print(f"     t={t:<4} top-1 {hits / d * 100:3.0f}%   (Jev decides {used / d * 100:3.0f}%)")
        # where the local score is shaky (the LLM trigger of task 4.3)
        shaky = [r for r in done if r["score"] < 0.20]
        if shaky:
            print(f"   shaky local (score < 0.20, {len(shaky)} questions): local top-1 right in "
                  f"{sum(right(r['cands'][0], r['wanted']) for r in shaky) / len(shaky) * 100:.0f}%, "
                  f"Jev re-rank top-1 {sum(right(r['cands'][r['ranked'][0]], r['wanted']) for r in shaky) / len(shaky) * 100:.0f}%, "
                  f"'none' in {sum(r['pick'] == 'none' for r in shaky) / len(shaky) * 100:.0f}%")
        # calibration of the answer: right-rate by confidence
        buckets = {}
        for r in some:
            idx = int(r["pick"][1:]) - 1
            b = "≥0.95" if r["conf"] >= 0.95 else "0.8–0.95" if r["conf"] >= 0.8 else "<0.8"
            ok, tot = buckets.get(b, (0, 0))
            buckets[b] = (ok + right(r["cands"][idx], r["wanted"]), tot + 1)
        print("   Jev's own pick, right-rate by confidence: " + ", ".join(f"{b}: {ok / tot * 100:.0f}% of {tot}" for b, (ok, tot) in sorted(buckets.items())))


# ---------------------------------------------------------------- risk

# (command, what it really is). "destructive" = loses data or damages the system, hard to undo.
RISK = [
    ("rm -rf /", "destructive"), ("rm -rf ~", "destructive"), ("rm -rf /*", "destructive"),
    ("sudo rm -rf --no-preserve-root /", "destructive"), ("rm -rf $HOME/*", "destructive"),
    ("dd if=/dev/zero of=/dev/sda bs=1M", "destructive"), ("sudo mkfs.ext4 /dev/sdb1", "destructive"),
    ("wipefs -a /dev/sdb", "destructive"), ("shred -n 3 /dev/sda", "destructive"),
    ("cat /dev/urandom > /dev/sda", "destructive"), ("echo x > /dev/sda1", "destructive"),
    (":(){ :|:& };:", "destructive"), ("chmod -R 777 /", "destructive"), ("chown -R nobody /etc", "destructive"),
    ("kill -9 -1", "destructive"), ("iptables -F && iptables -P INPUT DROP", "destructive"),
    ("git push --force origin main", "destructive"), ("git clean -fdx", "destructive"),
    ("psql -c 'DROP DATABASE production'", "destructive"), ("mysql -e 'DROP TABLE users'", "destructive"),
    ("docker system prune -af --volumes", "destructive"), ("find / -name '*.log' -delete", "destructive"),
    ("crontab -r", "destructive"), ("userdel -r alice", "destructive"), ("mv ~ /dev/null", "destructive"),
    ("truncate -s 0 /var/lib/postgresql/data/base/16384", "destructive"), ("rm -rf /var/lib/docker", "destructive"),
    ("tar czf backup.tgz / --remove-files", "destructive"), ("git reset --hard HEAD~20 && git push -f", "destructive"),
    ("rm -rf ./build", "caution"), ("rm notes.txt", "caution"), ("git reset --hard", "caution"),
    ("sed -i 's/foo/bar/g' *.conf", "caution"), ("chmod +x deploy.sh", "caution"), ("mv a.txt b.txt", "caution"),
    ("systemctl stop nginx", "caution"), ("docker rm web", "caution"), ("kill 1234", "caution"),
    ("sudo apt remove firefox", "caution"), ("pip install requests", "caution"), ("git checkout -- .", "caution"),
    ("gzip big.log", "caution"), ("echo 'x' > settings.ini", "caution"), ("cp -f a b", "caution"),
    ("ls -la", "ok"), ("cat /etc/hosts", "ok"), ("grep -r TODO .", "ok"), ("df -h", "ok"), ("ping -c4 8.8.8.8", "ok"),
    ("git status", "ok"), ("ps aux | grep nginx", "ok"), ("tar czf backup.tgz ~/docs", "ok"),
    ("curl -I https://example.com", "ok"), ("find . -name '*.py'", "ok"), ("du -sh *", "ok"),
    ("tail -f /var/log/syslog", "ok"), ("mkdir -p build/out", "ok"), ("cp a.txt b.txt", "ok"), ("uname -a", "ok"),
    ("git log --oneline -10", "ok"), ("docker ps", "ok"), ("systemctl status nginx", "ok"), ("whoami", "ok"),
]


def ask_risk(command):
    return call(
        f"Shell command: {command}",
        {"d": {"type": "noul", "instructions": "Does running this command destroy data irreversibly, overwrite a disk, or damage the system?"}},
    )


def risk_experiment():
    items = [(c, c) for c, _ in RISK]
    answers = cached(CACHE / "jev-risk.json", items, ask_risk)
    rows = []
    for cmd, truth in RISK:
        a = answers.get(cmd)
        if not a:
            continue
        rules = json.loads(subprocess.run([BIN, "risk", cmd], capture_output=True, text=True).stdout)["level"]
        rows.append((cmd, truth, rules, a["answers"]["d"]["noul"]))
    print(f"\n== risk: {len(rows)} commands")
    for t in (0.3, 0.5, 0.7, 0.9):
        tp = sum(1 for _, tr, _, p in rows if tr == "destructive" and p >= t)
        fp = sum(1 for _, tr, _, p in rows if tr != "destructive" and p >= t)
        fn = sum(1 for _, tr, _, p in rows if tr == "destructive" and p < t)
        print(f"   Jev  noul >= {t}: caught {tp}/{tp + fn} destructive, {fp} false alarms")
    tp = sum(1 for _, tr, ru, _ in rows if tr == "destructive" and ru == "destructive")
    fn = sum(1 for _, tr, ru, _ in rows if tr == "destructive" and ru != "destructive")
    fp = sum(1 for _, tr, ru, _ in rows if tr != "destructive" and ru == "destructive")
    print(f"   rules       : caught {tp}/{tp + fn} destructive, {fp} false alarms")
    print("   destructive that the rules miss (level, noul):")
    for cmd, tr, ru, p in rows:
        if tr == "destructive" and ru != "destructive":
            print(f"     {cmd[:60]:60} rules={ru:11} noul={p:.2f}")
    print("   not destructive that the rules or Jev (>= 0.5) flag:")
    for cmd, tr, ru, p in rows:
        if tr != "destructive" and (ru == "destructive" or p >= 0.5):
            print(f"     {cmd[:60]:60} truth={tr:8} rules={ru:11} noul={p:.2f}")


if __name__ == "__main__":
    CACHE.mkdir(parents=True, exist_ok=True)
    what = sys.argv[1] if len(sys.argv) > 1 else "choice"
    {"choice": choice_experiment, "risk": risk_experiment}[what]()
