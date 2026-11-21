#!/usr/bin/env python3
"""End-to-end checks of pistasho in a real zsh and a real tmux, on a pseudo-terminal.

The unit tests cannot see what a terminal shows, so this drives the installed binary the way a
user does (Ctrl-Space, typing, Enter, y, Esc…) and reads the screen back.

Hermetic: it uses its own XDG directories with a small cheat sheet, and fake `sudo`, `apt`,
`apt-cache` and `lsblk` at the front of PATH. Nothing is installed, no disk is touched, and the
user's config, database and history are not read. Commands that would be destructive are only
ever typed on the prompt; the script never presses Enter on them.

Needs: tmux, zsh, a Debian-like machine (the fake `apt-cache` stands in for the real one), and
the binary (`cargo install --path .`, or PISTASHO_BIN=target/debug/pistasho).

    python3 scripts/e2e.py            # everything
    python3 scripts/e2e.py install    # only scenarios whose name contains "install"
"""
import fcntl
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time

BIN = os.environ.get("PISTASHO_BIN") or os.path.expanduser("~/.cargo/bin/pistasho")
ROOT = tempfile.mkdtemp(prefix="pistasho-e2e-")
STUB = f"{ROOT}/stub"
LOG = f"{ROOT}/log"
FAILS: list[str] = []

CHEATS = """
[[cheat]]
cmd = "uptime --pretty"
desc = "cuanto tiempo lleva encendido el sistema"

[[cheat]]
cmd = "ncdu {{dir}}"
desc = "ver cuanto espacio ocupa cada carpeta"

[[cheat]]
cmd = "dust {{dir}}"
desc = "mostrar el uso de disco con dust"

[[cheat]]
cmd = "sudo mkfs.ext4 {{dispositivo}}"
desc = "formatear la usb en ext4"

[[cheat]]
cmd = "rm -rf build"
desc = "borrar la carpeta build entera"
"""

LSBLK = """{"blockdevices":[
 {"name":"sda","model":"Samsung SSD 860","size":"500G","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/sda",
  "children":[{"name":"sda1","model":null,"size":"500G","fstype":"ext4","mountpoint":"/","rm":false,"path":"/dev/sda1"}]},
 {"name":"sdb","model":"SanDisk Ultra","size":"29.8G","fstype":null,"mountpoint":null,"rm":true,"path":"/dev/sdb",
  "children":[{"name":"sdb1","model":null,"size":"29.8G","fstype":"vfat","mountpoint":"/media/pk/USB","rm":true,"path":"/dev/sdb1"}]}
]}"""


def setup():
    for d in (STUB, f"{ROOT}/cfg/pistasho", f"{ROOT}/data"):
        os.makedirs(d, exist_ok=True)
    open(f"{ROOT}/cfg/pistasho/config.toml", "w").write('language = "en"\n')
    open(f"{ROOT}/cfg/pistasho/cheats.toml", "w").write(CHEATS)
    fake("sudo", f'echo "[sudo] $*" >> {LOG}\nexec "$@"')
    fake("apt", f'''echo "[apt] $*" >> {LOG}
echo "Reading package lists... Done"
if [ -n "$FAKE_APT_FAIL" ]; then echo "E: Unable to locate package ncdu"; exit 100; fi
printf '#!/bin/sh\\necho ncdu\\n' > {STUB}/ncdu; chmod +x {STUB}/ncdu
echo "Setting up ncdu (1.19) ... done"''')
    # only `ncdu` is a package here; `dust` is not (so its box offers only the alternative)
    fake("apt-cache", 'for a in "$@"; do last="$a"; done\n[ "$last" = ncdu ] && printf "ncdu:\\n  Installed: (none)\\n  Candidate: 1.19\\n"; exit 0')
    fake("lsblk", f"cat <<'JSON'\n{LSBLK}\nJSON")


def fake(name, body):
    p = f"{STUB}/{name}"
    open(p, "w").write("#!/bin/sh\n" + body + "\n")
    os.chmod(p, 0o755)


def reset():
    for f in ("ncdu",):
        if os.path.exists(f"{STUB}/{f}"):
            os.remove(f"{STUB}/{f}")
    open(LOG, "w").close()
    os.environ.pop("FAKE_APT_FAIL", None)


ENV = {
    "HISTFILE": "/dev/null",
    "XDG_CONFIG_HOME": f"{ROOT}/cfg",
    "XDG_DATA_HOME": f"{ROOT}/data",
    "PATH": f"{STUB}:{os.path.dirname(BIN)}:{os.environ['PATH']}",
}


class Tmux:
    """A tmux server of its own, driven with send-keys, plus an optional real client on a pty."""

    def __init__(self, name):
        self.sock = f"e2e-{name}-{os.getpid()}"
        self.env = dict(os.environ, **ENV)
        self.fd = None
        self.raw = bytearray()

    def run(self, *args):
        return subprocess.run(["tmux", "-L", self.sock, *args], capture_output=True, text=True, env=self.env)

    def start(self, width=120, height=34, conf=None, shell="sh"):
        self.run("kill-server")
        time.sleep(0.2)
        base = ["tmux", "-L", self.sock] + (["-f", conf] if conf else [])
        subprocess.run(base + ["new-session", "-d", "-x", str(width), "-y", str(height), "-s", "s", shell], env=self.env)
        time.sleep(0.4)
        self.pane = self.run("list-panes", "-t", "s", "-F", "#{pane_id}").stdout.split()[0]
        if conf:  # a real client, so display-popup has somewhere to draw
            pid, self.fd = pty.fork()
            if pid == 0:
                os.environ.update(self.env)
                os.environ["TERM"] = "xterm-256color"
                os.execvp("tmux", base + ["attach", "-t", "s"])
            fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            self.drain(1.0)

    def stop(self):
        self.run("kill-server")

    def drain(self, t):
        end = time.time() + t
        while time.time() < end:
            if self.fd is None:
                time.sleep(0.05)
                continue
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                try:
                    self.raw.extend(os.read(self.fd, 65536))
                except OSError:
                    return

    def keys(self, *keys, wait=0.7):
        if self.fd is not None and keys and keys[0] == "POPUP":
            os.write(self.fd, b"\x02g")
        else:
            self.run("send-keys", "-t", self.pane, *keys)
        self.drain(wait) if self.fd is not None else time.sleep(wait)

    def type(self, text, wait=0.4):
        """Literal text (to the pane, or through the client when there is one)."""
        if self.fd is not None:
            os.write(self.fd, text.encode())
            self.drain(wait)
        else:
            self.run("send-keys", "-t", self.pane, "-l", text)
            time.sleep(wait)

    def send(self, raw, wait=0.7):
        """Raw bytes through the client (needed to reach a popup)."""
        os.write(self.fd, raw)
        self.drain(wait)

    def lines(self, color=False):
        args = ["capture-pane", "-p"] + (["-e"] if color else []) + ["-t", self.pane]
        return [l.rstrip() for l in self.run(*args).stdout.splitlines() if l.strip()]

    def text(self):
        return "\n".join(self.lines())

    def popup_text(self):
        return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-B0-2]|\x1b[=>]", "", self.raw.decode("utf8", "replace"))


def check(name, ok, detail=""):
    print(f"  [{'OK' if ok else 'FAIL'}] {name}" + (f"  — {detail}" if detail and not ok else ""))
    if not ok:
        FAILS.append(name)


def zsh_session(t, prompt='PROMPT> ', rprompt="[rp]", extra=""):
    t.start(shell="sh")
    env = " ".join(f"{k}={v}" for k, v in ENV.items())
    t.keys(f"env {env} zsh -f", "Enter", wait=0.9)
    t.keys(f'PS1="{prompt}"; RPROMPT="{rprompt}"; {extra} eval "$(pistasho init zsh)"; clear', "Enter", wait=1.0)


def ask(t, question, wait=1.8):
    t.keys("C-Space", wait=0.6)
    t.type(question)
    t.keys("Enter", wait=wait)


# ---------------------------------------------------------------- scenarios

def scenario_question():
    print("question and alternatives")
    t = Tmux("q")
    zsh_session(t)
    ask(t, "cuanto tiempo lleva encendido el sistema", 1.4)
    check("the best command lands on the prompt", "PROMPT> uptime --pretty" in t.text())
    check("with the line under the prompt", "⚡ local" in t.text())
    t.keys("C-u")
    t.type("uptime")
    t.keys("C-Space", wait=0.7)
    check("text already on the prompt becomes the question", "✦ ask  uptime" in t.text())
    t.keys("Escape", wait=0.6)
    check("Esc leaves the prompt as it was", t.lines()[-1].startswith("PROMPT> uptime"))
    t.stop()


def scenario_install():
    print("install")
    for lines, prompt in ((1, "PROMPT> "), (2, "~/proyecto\\nPROMPT> "), (3, "~/a\\n~/b\\nPROMPT> ")):
        reset()
        t = Tmux("i")
        zsh_session(t, prompt=f"$(printf '{prompt}')" if lines > 1 else prompt)
        ask(t, "ver cuanto espacio ocupa cada carpeta")
        check(f"[{lines} line prompt] the box appears", "suggested installation" in t.text())
        check("it shows the exact command", "sudo apt install -y ncdu" in t.text())
        check("nothing is installed yet", not os.path.exists(f"{STUB}/ncdu"))
        t.keys("Enter", wait=0.6)
        check("Enter does not confirm", not os.path.exists(f"{STUB}/ncdu") and "suggested installation" in t.text())
        if lines == 1:
            t.keys("Escape", wait=0.8)
            check("Esc cancels, nothing installed", not os.path.exists(f"{STUB}/ncdu"))
            ask(t, "ver cuanto espacio ocupa cada carpeta")
        t.keys("y", wait=1.6)
        check("y installs (the fake manager ran)", os.path.exists(f"{STUB}/ncdu"))
        t.type("/tmp")
        t.keys("Enter", wait=1.2)
        out = t.lines()
        check("the command is on the prompt", any(l.startswith("PROMPT> ncdu /tmp") for l in out))
        check("with ✓ installed", "✓ ncdu installed" in t.text())
        # the output of the manager must still be there, above the prompt, whatever the prompt height
        check("the installer output is not overwritten", "Setting up ncdu (1.19) ... done" in t.text())
        t.stop()
    reset()
    t = Tmux("i")
    zsh_session(t, extra="export FAKE_APT_FAIL=1;")
    ask(t, "ver cuanto espacio ocupa cada carpeta")
    t.keys("y", wait=1.6)
    last_prompt = [l for l in t.lines() if l.startswith("PROMPT>")][-1]
    check("a failing install leaves the prompt empty", "ncdu" not in last_prompt)
    check("and says so", "✗ installation failed" in t.text())
    t.stop()


def scenario_alternative():
    print("alternative without installing")
    reset()
    t = Tmux("a")
    zsh_session(t)
    ask(t, "ver cuanto espacio ocupa cada carpeta")
    check("the box offers the alternative", "[a] alternative without installing" in t.text() and "du -h -d1" in t.text())
    t.keys("a", wait=1.0)
    t.type("/tmp")
    t.keys("Enter", wait=1.2)
    check("the stand-in is on the prompt", any(l.startswith("PROMPT> du -h -d1 /tmp | sort -h") for l in t.lines()))
    check("nothing was run", open(LOG).read() == "")
    t.keys("C-u", wait=0.4)  # a clean line: with text on it, Ctrl-Space would open the alternatives
    ask(t, "mostrar el uso de disco con dust")
    check("with no package, only the alternative is offered", "[a]" in t.text() and "[y]" not in t.text())
    t.keys("y", wait=0.6)
    check("and y does nothing there", "suggested installation" in t.text())
    t.stop()


def scenario_risk():
    print("risk: red box, prompt mark, device picker")
    red = lambda l: any(c in l for c in ("31m", "38;5;1m", "91m", "38;2;247;118;142m"))
    t = Tmux("r")
    zsh_session(t)
    ask(t, "formatear la usb en ext4")
    check("the device list appears", "device (lsblk)" in t.text())
    check("only the removable device is offered", "/dev/sdb1" in t.text() and "sda hidden: not removable" in t.text())
    t.type("xyz")
    check("typing changes nothing there", "device (lsblk)" in t.text())
    t.keys("Enter", wait=1.8)
    check("the red box appears", "destructive command" in t.text())
    check("it names the device from lsblk", "SanDisk Ultra" in t.text() and "mounted at /media/pk/USB" in t.text())
    check("and the fixed rule", "fixed rule mkfs" in t.text())
    t.keys("Enter", wait=1.2)
    prompt_line = [l for l in t.lines(color=True) if l.startswith("PROMPT>")][-1]
    check("the command is on the prompt", "sudo mkfs.ext4 /dev/sdb1" in re.sub(r"\x1b\[[0-9;]*m", "", prompt_line))
    check("painted red", red(prompt_line))
    check("marked ⚠ destructive on the right", "⚠ destructive" in t.text())
    check("with the review message", "review before running" in t.text())
    t.keys("x", wait=0.5)
    prompt_line = [l for l in t.lines(color=True) if l.startswith("PROMPT>")][-1]
    check("editing it takes the mark and the paint away", not red(prompt_line) and "⚠" not in prompt_line)
    t.keys("C-u", wait=0.4)
    ask(t, "formatear la usb en ext4")
    t.keys("Enter", wait=1.8)
    t.keys("Escape", wait=1.0)
    last_prompt = [l for l in t.lines() if l.startswith("PROMPT>")][-1]
    check("Esc in the red box leaves the prompt empty", "mkfs" not in last_prompt and "⚠" not in last_prompt)
    ask(t, "formatear la usb en ext4")
    t.keys("Enter", wait=1.8)
    t.keys("e", wait=0.8)
    check("e opens the editor", "✎ edit" in t.text())
    t.keys("C-u", wait=0.2)
    t.type("sudo mkfs.ext4 /dev/sdz9")
    t.keys("Enter", wait=1.5)
    check("the edited command is checked again", "destructive command" in t.text() and "not found on this machine" in t.text())
    t.keys("Escape", wait=0.8)
    t.keys("C-u", wait=0.3)
    ask(t, "borrar la carpeta build entera", 1.6)
    check("caution: no box, marked, with the effect", "destructive command" not in t.text() and "⚠ caution" in t.text())
    t.keys("C-u", wait=0.4)
    ask(t, "cuanto tiempo lleva encendido el sistema", 1.4)
    check("ok: nothing marked", "⚠" not in t.text())
    t.stop()


def scenario_popup():
    print("tmux popup")
    reset()
    conf = f"{ROOT}/tmux.conf"
    subprocess.run([BIN, "init", "tmux"], stdout=open(conf, "w"), env=dict(os.environ, **ENV))
    t = Tmux("p")
    t.start(conf=conf, shell="zsh -f")
    t.type('PS1="PROMPT> "; clear\r', wait=0.8)

    def popup(question, enters=1, wait=2.0):
        t.raw.clear()
        t.send(b"\x02g", 1.8)
        t.type(question)
        t.send(b"\r", wait)

    popup("cuanto tiempo lleva encendido el sistema")
    check("question → list of the answer", "other options" in t.popup_text())
    t.send(b"\r", 1.2)
    check("Enter sends the command to the pane, without Enter", "PROMPT> uptime --pretty" in t.text())
    t.type("\x15", 0.3)
    reset()
    popup("ver cuanto espacio ocupa cada carpeta")
    check("the install box shows in the popup", "suggested installation" in t.popup_text())
    t.send(b"y", 2.2)
    check("y installs", os.path.exists(f"{STUB}/ncdu"))
    t.send(b"x", 1.0)  # a key closes the notice
    t.send(b"\r", 1.0)
    t.type("/var")
    t.send(b"\r", 1.5)
    check("the command reaches the pane", "PROMPT> ncdu /var" in t.text())
    t.type("\x15", 0.3)
    popup("formatear la usb en ext4")
    t.send(b"\r", 1.5)  # the list
    check("device list in the popup", "device (lsblk)" in t.popup_text())
    t.send(b"\r", 1.8)
    check("red box in the popup", "destructive command" in t.popup_text())
    check("nothing is sent before confirming", "mkfs" not in t.text())
    t.send(b"\r", 1.5)
    check("then it arrives, typed and not run", "PROMPT> sudo mkfs.ext4 /dev/sdb1" in t.text())
    t.type("\x15", 0.3)
    popup("formatear la usb en ext4")
    t.send(b"\r", 1.5)
    t.send(b"\r", 1.8)
    t.send(b"\x1b", 1.5)
    check("Esc in the red box sends nothing", "mkfs" not in t.text())
    t.stop()


SCENARIOS = [scenario_question, scenario_install, scenario_alternative, scenario_risk, scenario_popup]


def main():
    if not os.path.exists(BIN):
        sys.exit(f"binary not found: {BIN} (run `cargo install --path .` or set PISTASHO_BIN)")
    for tool in ("tmux", "zsh"):
        if not shutil.which(tool):
            sys.exit(f"{tool} is required")
    setup()
    wanted = sys.argv[1:]
    try:
        for s in SCENARIOS:
            if wanted and not any(w in s.__name__ for w in wanted):
                continue
            reset()
            s()
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)
    print()
    if FAILS:
        print(f"{len(FAILS)} failed:")
        for f in FAILS:
            print("  -", f)
        sys.exit(1)
    print("all checks passed")


if __name__ == "__main__":
    main()
