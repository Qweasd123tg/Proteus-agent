#!/usr/bin/env python3
"""Stdio peer that emits output while the host is waiting for user interaction."""
import json
import os
import pathlib
import sys
import time

os.chdir(sys.argv[sys.argv.index("--cwd") + 1])


def emit(value):
    print(json.dumps(value), flush=True)


def result(request_id, text):
    emit({"type": "response", "id": request_id, "ok": True,
          "output": {"text": text, "metadata": None}, "error": None})


for line in sys.stdin:
    request = json.loads(line)
    kind = request["type"]
    if kind == "send":
        active_id = request["id"]
        mode = request["text"]
        if mode == "healthy":
            result(active_id, "healthy peer")
            continue
        if mode == "input-overflow":
            event = {"type": "user_input_requested", "request": {
                "request_id": "input", "cwd": str(pathlib.Path.cwd()),
                "title": None, "questions": [], "origin": None, "seq": 0}}
        else:
            event = {"type": "approval_requested", "request": {
                "approval_id": "approval", "call": {
                    "id": "call", "name": "shell", "args": {},
                    "surface": "function", "raw_arguments": None},
                "cwd": str(pathlib.Path.cwd()), "reason": "fixture",
                "tool_spec": None, "preview": None, "origin": None, "seq": 0}}
        emit({"type": "event", "event": event})
        while not pathlib.Path("waiting").exists():
            time.sleep(0.005)
        pathlib.Path("flood-started").touch()
        if mode == "stdout-closed":
            sys.exit(0)
        if mode == "malformed-output":
            print('{"type":"unknown"}', flush=True)
            continue
        if mode == "frame-overflow":
            # No newline: the limit must apply before an entire line is read.
            sys.stdout.write("x" * (9 * 1024 * 1024))
            sys.stdout.flush()
        else:
            count = 300 if "overflow" in mode else 32
            size = 1024 * 1024 if mode == "bytes-overflow" else 64
            for index in range(count):
                if mode == "normal":
                    identity = "00000000-0000-0000-0000-000000000001"
                    event = {"type": "runtime", "envelope": {
                        "schema_version": 2, "event_id": identity,
                        "session_id": identity, "thread_id": identity,
                        "turn_id": identity, "seq": index, "timestamp_ms": 0,
                        "event": {"AssistantTextDelta": {
                            "offset": index * 3, "message_id": identity,
                            "phase": None, "text": f"{index:02};"}}}}
                else:
                    event = {"type": "user_message_submitted", "text": "x" * size}
                emit({"type": "event", "event": event})
        pathlib.Path("produced").touch()
    elif kind in ("approval", "user_input"):
        pathlib.Path("answered").touch()
        result(active_id, "approved result")
    elif kind == "cancel":
        pathlib.Path("cancelled").touch()
        result(request["target_id"], "cancelled result")
    elif kind == "clear_history":
        result(request["id"], "cleared")
