#!/usr/bin/env python3
"""Second smoke test: detach/reattach cwd restoration, `termnote note` and
`termnote bookmark` invoked *from inside* a running session (PRD §103-104),
and export.
"""
import json
import os
import select
import sqlite3
import sys
import time
import pty

TERMNOTE_BIN = os.path.abspath("target/debug/termnote")
DB_PATH = "/tmp/termnote-test-data/termnote/termnote.db"
SESSION_NAME = "e2e-reattach"

os.environ["TERMNOTE_CONFIG"] = "/tmp/termnote-test-config2.toml"
os.environ["XDG_DATA_HOME"] = "/tmp/termnote-test-data"
os.environ["PATH"] = os.path.dirname(TERMNOTE_BIN) + ":" + os.environ.get("PATH", "")
os.environ.pop("SHELL", None)

for p in (DB_PATH, "/tmp/termnote-test-config2.toml"):
    if os.path.exists(p):
        os.remove(p)


def run_in_pty(argv, steps, settle=1.0):
    pid, master_fd = pty.fork()
    if pid == 0:
        os.execv(argv[0], argv)
        os._exit(127)

    def drain(timeout=0.3):
        while True:
            r, _, _ = select.select([master_fd], [], [], timeout)
            if not r:
                return
            try:
                chunk = os.read(master_fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            sys.stdout.write(f"[pty-out] {chunk!r}\n")

    time.sleep(settle)
    drain()
    for text, wait in steps:
        os.write(master_fd, text.encode())
        time.sleep(wait)
        drain()

    for _ in range(30):
        try:
            os.kill(pid, 0)
            time.sleep(0.2)
        except OSError:
            break
    _, status = os.waitpid(pid, 0)
    return status


status = run_in_pty(
    [TERMNOTE_BIN, "new", SESSION_NAME],
    [("echo one\n", 0.8), ("exit\n", 1.2)],
)
print("phase1 exit status:", status)

status = run_in_pty(
    [TERMNOTE_BIN, "attach", SESSION_NAME],
    [
        ("export EDITOR=/tmp/termnote-test-bin/fake-editor.sh\n", 0.6),
        ("termnote note\n", 1.0),
        ('termnote bookmark "checkpoint one"\n', 1.0),
        ("pwd\n", 0.8),
        ("exit\n", 1.2),
    ],
)
print("phase2 exit status:", status)

con = sqlite3.connect(DB_PATH)
cur = con.cursor()
cur.execute("SELECT id FROM sessions WHERE name = ?", (SESSION_NAME,))
(session_id,) = cur.fetchone()

cur.execute("SELECT sequence, type, payload FROM events WHERE session_id = ? ORDER BY sequence", (session_id,))
rows = cur.fetchall()
print(f"\n{len(rows)} events:")
types_seen = []
note_found = False
bookmark_found = False
for seq, etype, payload in rows:
    print(f"  #{seq:<3} {etype:<16} {payload[:100]}")
    types_seen.append(etype)
    if etype == "NOTE":
        note_found = True
        data = json.loads(payload)
        assert "Auto note" in data["markdown"], data
    if etype == "BOOKMARK":
        bookmark_found = True
        data = json.loads(payload)
        assert data["name"] == "checkpoint one", data

assert "SESSION_ATTACH" in types_seen, "reattach should record a SESSION_ATTACH event"
assert note_found, "termnote note (run inside the session) should have created a NOTE event"
assert bookmark_found, "termnote bookmark (run inside the session) should have created a BOOKMARK event"

cur.execute(
    "SELECT payload FROM events WHERE session_id = ? AND type = 'COMMAND' ORDER BY sequence",
    (session_id,),
)
commands = [json.loads(r[0]) for r in cur.fetchall()]
pwds = [c for c in commands if c["command"] == "pwd"]
echoes = [c for c in commands if c["command"] == "echo one"]
assert pwds and echoes, commands
print("\necho cwd:", echoes[0]["cwd"])
print("pwd  cwd:", pwds[0]["cwd"])
assert pwds[0]["cwd"] == echoes[0]["cwd"], "reattach should have restored the working directory"

export_status = os.system(f"{TERMNOTE_BIN} export {SESSION_NAME} --format markdown -o /tmp/e2e-export.md")
assert export_status == 0
md = open("/tmp/e2e-export.md").read()
print("\n--- exported markdown (first 800 chars) ---")
print(md[:800])
assert "echo one" in md
assert "Auto note" in md
assert "checkpoint one" in md

print("\nALL ASSERTIONS PASSED")
