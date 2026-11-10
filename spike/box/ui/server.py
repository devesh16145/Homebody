#!/usr/bin/env python3
"""Box Console — a tiny local web UI over the running Box (spike quality, stdlib only).

Serves http://127.0.0.1:8787 with:
  - status: is sai-box running, is the vsock SSH bridge answering, which destinations are trusted
  - live feed of receipts written by the Box proxy (allowed / denied, with rule)
  - "Run in Box": runs a Claude Code task inside the guest over the SSH bridge (claude -p)
  - "Try to exfiltrate": curl to webhook.site from inside the guest, which the proxy must deny
  - verify: runs fencelog verify on the receipt chain

The API key typed into the page is kept in this process's memory only and is passed to the guest
as an environment variable over the vsock SSH tunnel. Nothing is written to disk.
"""
import json, os, shlex, subprocess, threading, time, uuid, re
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

HERE = os.path.dirname(os.path.abspath(__file__))
BOX = os.path.dirname(HERE)
BOX_SSH = os.path.join(BOX, "box-ssh.sh")
FENCELOG = os.path.join(BOX, "..", "fence", "fencelog", ".build", "release", "fencelog")
RECEIPTS = os.path.expanduser("~/Library/Application Support/SandboxAI/receipts.jsonl")
PORT = int(os.environ.get("BOX_UI_PORT", "8787"))
RUNNER = os.environ.get("SAI_RUNNER", "agent")          # "agent" = fenced macOS user, "ssh" = Box VM
AGENT_FENCE = os.path.join(BOX, "..", "fence", "agent", "agent-fence.sh")
AGENT_USER = os.environ.get("AGENT_USER", "_saiagent")
WORK = os.environ.get("WORK", "/Users/Shared/sandbox-ai/work")
PROXY_PORT = os.environ.get("PROXY_PORT", "3128")

jobs = {}
jobs_lock = threading.Lock()


def sh(cmd, timeout=20):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout).stdout
    except Exception as e:
        return ""


def agent_cmd(remote_cmd):
    """Run a shell command as the fenced user, through the proxy, in the workspace."""
    env = ["HOME=/var/sai/agent", f"USER={AGENT_USER}", "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
           f"HTTPS_PROXY=http://127.0.0.1:{PROXY_PORT}", f"HTTP_PROXY=http://127.0.0.1:{PROXY_PORT}",
           f"https_proxy=http://127.0.0.1:{PROXY_PORT}", f"http_proxy=http://127.0.0.1:{PROXY_PORT}",
           "NO_PROXY=localhost,127.0.0.1", "DISABLE_AUTOUPDATER=1", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1"]
    return ["sudo", "-n", "-u", AGENT_USER, "-H", "env", "-i", *env, "/bin/zsh", "-c", f"cd {shlex.quote(WORK)} && {remote_cmd}"]


def runner_argv(remote_cmd):
    return agent_cmd(remote_cmd) if RUNNER == "agent" else [BOX_SSH, remote_cmd]


def vm_status():
    if RUNNER == "agent":
        proxy = sh(["pgrep", "-f", "sai-box proxy"]).strip().split("\n")[0]
        allow = re.findall(r"--allow\s+(\S+)", sh(["ps", "-o", "args=", "-p", proxy])) if proxy else []
        user_ok = subprocess.run(["id", AGENT_USER], capture_output=True).returncode == 0
        try:
            r = subprocess.run(runner_argv("true"), capture_output=True, timeout=8)
            run_ok = r.returncode == 0
        except Exception:
            run_ok = False
        return {"mode": "agent", "vm": bool(proxy) and user_ok, "ssh": run_ok, "allow": allow,
                "detail": ("proxy running" if proxy else "proxy NOT running") + " · " +
                          (f"user {AGENT_USER} ready" if user_ok else f"user {AGENT_USER} missing") + " · " +
                          ("console may run commands" if run_ok else "console cannot run commands: sudo ./agent-fence.sh allow-console")}
    pid = sh(["pgrep", "-x", "sai-box"]).strip().split("\n")[0]
    allow = []
    if pid:
        args = sh(["ps", "-o", "args=", "-p", pid])
        allow = re.findall(r"--allow\s+(\S+)", args)
    ssh_ok = False
    if pid:
        try:
            r = subprocess.run([BOX_SSH, "true"], capture_output=True, timeout=8)
            ssh_ok = r.returncode == 0
        except Exception:
            ssh_ok = False
    return {"mode": "ssh", "vm": bool(pid), "ssh": ssh_ok, "allow": allow,
            "detail": "Box running · SSH bridge up · no network interface" if (pid and ssh_ok) else ("Box running · SSH bridge not answering yet" if pid else "Box is not running")}


def receipts(limit=200):
    out = []
    try:
        with open(RECEIPTS) as f:
            lines = f.readlines()
    except FileNotFoundError:
        return out
    for l in lines[-2000:]:
        try:
            b = json.loads(l)["body"]
        except Exception:
            continue
        if b.get("source") != "box":
            continue
        out.append({"ts": b["ts"], "decision": b["decision"], "host": b.get("dstHost") or b["dstIP"],
                    "port": b["dstPort"], "rule": b["rule"], "process": b["process"]})
    return out[-limit:]


def start_job(remote_cmd):
    jid = uuid.uuid4().hex[:8]
    job = {"id": jid, "output": "", "done": False, "started": time.time(), "code": None}
    with jobs_lock:
        jobs[jid] = job

    def run():
        try:
            p = subprocess.Popen(runner_argv(remote_cmd), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            for line in p.stdout:
                job["output"] += line
            p.wait()
            job["code"] = p.returncode
        except Exception as e:
            job["output"] += f"\n[ui] failed to run: {e}\n"
            job["code"] = -1
        job["done"] = True

    threading.Thread(target=run, daemon=True).start()
    return jid


def build_command(kind, text, api_key):
    if kind == "exfil":
        return ("echo 'fenced: curl https://webhook.site/sandbox-ai-test'; "
                "curl -sS -m 10 https://webhook.site/sandbox-ai-test; echo \"exit=$?\"")
    if kind == "allowed":
        return ("echo 'fenced: curl https://api.anthropic.com/'; "
                "curl -sS -m 10 -o /dev/null -w 'http %{http_code}\\n' https://api.anthropic.com/; echo \"exit=$?\"")
    if kind == "shell":
        return text
    if kind == "task":
        env = f"ANTHROPIC_API_KEY={shlex.quote(api_key)} " if api_key else ""
        cd = "" if RUNNER == "agent" else "cd /work/spike/box/playground && "
        return (f"{cd}{env}"
                f"claude -p --dangerously-skip-permissions --output-format text {shlex.quote(text)} < /dev/null 2>&1; echo \"exit=$?\"")
    raise ValueError("unknown kind")


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):  # quiet
        pass

    def _json(self, obj, code=200):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        u = urlparse(self.path)
        if u.path == "/":
            with open(os.path.join(HERE, "index.html"), "rb") as f:
                data = f.read()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        elif u.path == "/api/status":
            self._json(vm_status())
        elif u.path == "/api/receipts":
            self._json(receipts())
        elif u.path.startswith("/api/job/"):
            jid = u.path.rsplit("/", 1)[1]
            with jobs_lock:
                job = jobs.get(jid)
            self._json(job or {"error": "no such job"}, 200 if job else 404)
        elif u.path == "/api/verify":
            out = sh([FENCELOG, "verify"], timeout=60) or "fencelog not built"
            self._json({"output": out.strip()})
        else:
            self._json({"error": "not found"}, 404)

    def do_POST(self):
        u = urlparse(self.path)
        n = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(n) or b"{}")
        if u.path == "/api/run":
            try:
                cmd = build_command(body.get("kind"), body.get("text", ""), body.get("apiKey", ""))
            except ValueError as e:
                return self._json({"error": str(e)}, 400)
            return self._json({"job": start_job(cmd)})
        self._json({"error": "not found"}, 404)


if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", PORT), H)
    print(f"Sandbox-ai Console ({RUNNER} mode) on http://127.0.0.1:{PORT}", flush=True)
    srv.serve_forever()
