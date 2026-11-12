#!/usr/bin/env python3
"""terminal.py — a protected program and the terminal it runs in.

    tests/terminal.py [path to homebody] [app id]

Needs a running engine and the fence set up for the app (default: claude-code). It runs
`homebody run <app> -- …` the two ways a person meets it and checks that the program never
outlives its terminal:

  - as the program a terminal starts (what an editor's terminal does, and what Homebody's own
    "Open" does), then stopped the way an editor stops it: "hang up" sent to that one process;
  - typed into an interactive shell, with Ctrl-Z, fg and Ctrl-C.

The first is the regression this file exists for: an editor that closed an agent's terminal
left the agent running unseen until the editor quit, because the agent ignores "hang up".
"""
import os
import pty
import re
import select
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
HB = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "target", "debug", "homebody"))
APP = sys.argv[2] if len(sys.argv) > 2 else "claude-code"
passed = failed = 0


def check(name, ok, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"  \033[32m✓\033[0m {name}")
    else:
        failed += 1
        print(f"  \033[31m✗ {name}\033[0m\n      {detail}")


def run(*command, timeout=30):
    return subprocess.run([HB, "run", APP, "--cwd", "/", "--", *command], capture_output=True, text=True, timeout=timeout)


def table():
    """Every process: (uid, state, command line)."""
    out = subprocess.run(["ps", "-axo", "uid=,stat=,command="], capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3:
            rows.append((parts[0], parts[1], parts[2]))
    return rows


def of_the_app(mark):
    """States of the protected user's processes whose command line contains `mark`."""
    return [state for uid, state, command in table() if uid == APP_UID and mark in command]


def keepers(mark):
    return [state for uid, state, command in table() if uid == str(os.getuid()) and command.startswith(HB + " run") and mark in command]


def gone_within(mark, seconds):
    end = time.time() + seconds
    while time.time() < end:
        if not of_the_app(mark) and not keepers(mark):
            return True
        time.sleep(0.3)
    return False


def in_a_terminal(mark, shell_line):
    """Start `homebody run` as the program of a new terminal device. Returns (pid, fd)."""
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(HB, [HB, "run", APP, "--cwd", "/", "--", "/bin/sh", "-c", f": {mark}; {shell_line}"])
    end = time.time() + 15
    while time.time() < end and not of_the_app(mark):
        drain(fd, 0.3)
    return pid, fd


def drain(fd, seconds):
    out = b""
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if ready:
            try:
                out += os.read(fd, 65536)
            except OSError:
                break
    return re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b"", out).decode(errors="replace")


def finish(pid, fd):
    try:
        os.close(fd)
    except OSError:
        pass
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except OSError:
        pass


# Homebody refuses to run a program while it is re-checking that the fence holds (it does that
# after a setting changes); give it a moment rather than calling that a failure.
for attempt in range(20):
    who = run("/usr/bin/id", "-u")
    APP_UID = who.stdout.strip()
    if who.returncode == 0 and APP_UID.isdigit():
        break
    time.sleep(1)
if who.returncode != 0 or not APP_UID.isdigit() or APP_UID == str(os.getuid()):
    print(f"{APP} cannot be run protected here: {who.stderr.strip() or who.stdout.strip()}")
    sys.exit(2)

print("Plain use")
r = run("/bin/sh", "-c", "exit 7")
check("its exit code comes back", r.returncode == 7, f"got {r.returncode}")
r = run("/bin/echo", "hello from inside")
check("its output comes back", r.stdout.strip() == "hello from inside", r.stdout + r.stderr)

print("An editor closes its terminal (hang up to the one process the terminal started)")
for name, mark, line, sig, limit in [
    ("a program that ignores hang up, as Claude Code does, is ended", "hb-term-a", "trap '' HUP; while :; do sleep 1; done", signal.SIGHUP, 6),
    ("a program that ignores hang up and stop is ended too", "hb-term-b", "trap '' HUP TERM; while :; do sleep 1; done", signal.SIGHUP, 10),
    ("an ordinary stop ends it", "hb-term-c", "while :; do sleep 1; done", signal.SIGTERM, 6),
]:
    pid, fd = in_a_terminal(mark, line)
    started = bool(of_the_app(mark))
    os.kill(pid, sig)
    began = time.time()
    ok = started and gone_within(mark, limit)
    check(name, ok, "it did not start" if not started else f"still running {limit} s later: {of_the_app(mark)} keeper {keepers(mark)}")
    if ok:
        took = time.time() - began
        check("  and nothing of it is left behind", not of_the_app(mark), f"after {took:.0f} s")
    finish(pid, fd)

print("Typed into a shell: Ctrl-Z, fg, Ctrl-C")
mark = "hb-term-d"
pid, fd = pty.fork()
if pid == 0:
    os.environ["PS1"] = "READY> "
    os.execv("/bin/bash", ["bash", "--norc", "--noprofile", "-i"])
drain(fd, 1.5)
os.write(fd, f"{HB} run {APP} --cwd / -- /bin/sh -c ': {mark}; while :; do sleep 1; done'\n".encode())
end = time.time() + 15
while time.time() < end and not of_the_app(mark):
    drain(fd, 0.3)
check("it starts", bool(of_the_app(mark)) and bool(keepers(mark)), f"{of_the_app(mark)} {keepers(mark)}")
os.write(fd, b"\x1a")
drain(fd, 3)
check("Ctrl-Z suspends it", any(s.startswith("T") for s in of_the_app(mark)) and all(s.startswith("T") for s in keepers(mark)), f"program {of_the_app(mark)} keeper {keepers(mark)}")
os.write(fd, b"echo shell-is-$((40+2))\n")
check("and the shell has the terminal back", "shell-is-42" in drain(fd, 2))
os.write(fd, b"fg\n")
drain(fd, 3)
check("fg brings it back, running", bool(of_the_app(mark)) and not any(s.startswith("T") for s in of_the_app(mark) + keepers(mark)), f"program {of_the_app(mark)} keeper {keepers(mark)}")
os.write(fd, b"\x03")
said = drain(fd, 4)
check("Ctrl-C ends it and the shell is back", gone_within(mark, 4) and "READY>" in said, f"program {of_the_app(mark)} keeper {keepers(mark)} shell said {said[-60:]!r}")
finish(pid, fd)

# Whatever happened above, leave nothing running.
for uid, state, command in table():
    if uid == str(os.getuid()) and command.startswith(HB + " run") and "hb-term-" in command:
        subprocess.run(["pkill", "-TERM", "-f", "hb-term-"], capture_output=True)
        break

print(f"\n{passed} passed, {failed} failed")
sys.exit(1 if failed else 0)
