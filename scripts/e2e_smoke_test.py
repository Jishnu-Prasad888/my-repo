#!/usr/bin/env python3
"""End-to-end smoke test: spawn `termnote new` inside a real PTY (like a
terminal emulator would), type some commands at it, and verify what ended
up in the SQLite database. This exercises the full stack: PTY spawn, raw
mode, the pgrp-based command-boundary poller, the auto-injected bash exit
hook, and the storage layer -- all the pieces that are hard to unit test in
isolation.
"""
import os
import pty
import signal
import sqlite3
import sys
import time

TERMNOTE_BIN = os.path.abspath("target/debug/termnote")
DB_PATH = "/tmp/termnote-test-data/termnote/termnote.db"
SESSION_NAME = "e2e-test"

os.environ["TERMNOTE_CONFIG"] = "/tmp/termnote-test-config.toml"
os.environ["XDG_DATA_HOME"] = "/tmp/termnote-test-data"
os.environ.pop("SHELL", None)

if os.path.exists(DB_PATH):
    os.remove(DB_PATH)

pid, master_fd = pty.fork()
if pid == 0:
    os.execv(TERMNOTE_BIN, [TERMNOTE_BIN, "new", SESSION_NAME])
    os._exit(127)

def send(s, wait=0.9):
    os.write(master_fd, s.encode())
    time.sleep(wait)
    drain()

import select

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

time.sleep(1.2)
drain()
send("echo hello-termnote\n")
send("false\n")
send("cd /tmp\n")
send("pwd\n")
send("exit\n", wait=1.5)

# Give the recorder time to flush its graceful shutdown.
for _ in range(20):
    try:
        os.kill(pid, 0)
        time.sleep(0.2)
    except OSError:
        break
else:
    os.kill(pid, signal.SIGKILL)

_, status = os.waitpid(pid, 0)
print(f"\nchild exit status: {status}")

assert os.path.exists(DB_PATH), "database was never created!"

con = sqlite3.connect(DB_PATH)
cur = con.cursor()

cur.execute("SELECT id, name, status FROM sessions WHERE name = ?", (SESSION_NAME,))
row = cur.fetchone()
print("session row:", row)
assert row is not None, "session was not created"
session_id, _, status = row
assert status == "DETACHED", f"expected DETACHED after exit, got {status}"

cur.execute(
    "SELECT sequence, type, payload FROM events WHERE session_id = ? ORDER BY sequence",
    (session_id,),
)
rows = cur.fetchall()
print(f"\n{len(rows)} events recorded:")
commands = []
for seq, etype, payload in rows:
    print(f"  #{seq:<3} {etype:<16} {payload[:120]}")
    if etype == "COMMAND":
        commands.append(payload)

assert any('"echo hello-termnote"' in c for c in commands), "echo command missing"
assert any('"false"' in c for c in commands), "false command missing"
assert any('"cd /tmp"' in c or '"cd/tmp"' in c for c in commands), "cd command missing"
assert any('"pwd"' in c for c in commands), "pwd command missing"

import json
echo_cmd = next(json.loads(c) for c in commands if json.loads(c)["command"] == "echo hello-termnote")
false_cmd = next(json.loads(c) for c in commands if json.loads(c)["command"] == "false")
cd_cmd = next(json.loads(c) for c in commands if json.loads(c)["command"] == "cd /tmp")
pwd_cmd = next(json.loads(c) for c in commands if json.loads(c)["command"] == "pwd")

print("\necho payload:", echo_cmd)
print("false payload:", false_cmd)
print("cd payload:", cd_cmd)
print("pwd payload:", pwd_cmd)

assert echo_cmd["exit_code"] == 0, f"echo should exit 0, got {echo_cmd}"
assert echo_cmd["closed"] is True
assert false_cmd["exit_code"] == 1, f"false should exit 1 (via shell-hook), got {false_cmd}"
assert cd_cmd["closed"] is True, "cd (builtin) should still be closed out"
assert pwd_cmd["cwd"] == "/tmp", f"pwd should report cwd=/tmp after `cd /tmp`, got {pwd_cmd}"

cur.execute("SELECT content FROM events_fts WHERE events_fts MATCH ?", ('"hello-termnote"',))
fts_hit = cur.fetchall()
print("\nFTS search for 'hello-termnote':", fts_hit)
assert fts_hit, "full-text search should find the echoed string in output"

print("\nALL ASSERTIONS PASSED")
