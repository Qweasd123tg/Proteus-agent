#!/usr/bin/env python3
"""Stateful newline-delimited MCP fixture used by mcp_client.rs."""

import json
import os
import sys
import threading
import time


EVENT_LOG = os.environ["MCP_FIXTURE_EVENT_LOG"]
GENERATION_FILE = os.environ["MCP_FIXTURE_GENERATION_FILE"]
INIT_MODE = os.environ.get("MCP_FIXTURE_INIT_MODE", "ok")
WRITE_LOCK = threading.Lock()
CANCEL_LOCK = threading.Lock()
CANCELLED = {}


def append(path, line):
    with open(path, "a", encoding="utf-8") as file:
        file.write(line + "\n")
        file.flush()


with open(GENERATION_FILE, "a+", encoding="utf-8") as generations:
    generations.seek(0)
    GENERATION = sum(1 for line in generations if line.strip()) + 1
    generations.write(f"{GENERATION}\n")
    generations.flush()


def event(name):
    append(EVENT_LOG, f"{GENERATION}:{name}")


def send(message):
    encoded = json.dumps(message, ensure_ascii=False, separators=(",", ":"))
    with WRITE_LOCK:
        sys.stdout.write(encoded + "\n")
        sys.stdout.flush()


def result(request_id, value):
    send({"jsonrpc": "2.0", "id": request_id, "result": value})


def error(request_id, code, message):
    send({"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}})


def tool(name, description, properties=None):
    return {
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties or {},
            "additionalProperties": False,
        },
    }


def call_tool(request_id, params):
    name = params.get("name")
    arguments = params.get("arguments", {})
    event(f"call_started:{request_id}:{name}")
    if name == "echo":
        if arguments.get("mode") == "rpc_error":
            error(request_id, -32001, "ordinary fixture RPC error")
            event(f"call_finished:{request_id}:{name}")
            return
        delay_ms = arguments.get("delay_ms", 0)
        time.sleep(delay_ms / 1000)
        label = arguments.get("label", "")
        result(request_id, {
            "content": [{"type": "text", "text": f"echo:{label}:generation:{GENERATION}"}],
            "structuredContent": {
                "label": label,
                "generation": GENERATION,
                "path_present": bool(os.environ.get("PATH")),
                "home_present": bool(os.environ.get("HOME")),
                "literal_env": os.environ.get("MCP_LITERAL_ENV"),
            },
            "isError": False,
        })
    elif name == "fail":
        result(request_id, {
            "content": [{"type": "text", "text": "fixture failure"}],
            "structuredContent": {"kind": "expected"},
            "isError": True,
        })
    elif name == "delay_write":
        delay_ms = arguments.get("delay_ms", 1000)
        deadline = time.monotonic() + delay_ms / 1000
        key = json.dumps(request_id, sort_keys=True)
        while time.monotonic() < deadline:
            with CANCEL_LOCK:
                if CANCELLED.get(key, False):
                    event(f"call_cancelled:{request_id}")
                    error(request_id, -32800, "request cancelled")
                    return
            time.sleep(0.01)
        append(arguments["marker"], f"generation:{GENERATION}")
        result(request_id, {"content": [{"type": "text", "text": "written"}], "isError": False})
    elif name == "oversize":
        result(request_id, {"content": [{"type": "text", "text": "x" * 100000}], "isError": False})
    else:
        error(request_id, -32602, f"unknown tool {name}")
    event(f"call_finished:{request_id}:{name}")


event("spawn")
for raw_line in sys.stdin:
    try:
        request = json.loads(raw_line)
    except json.JSONDecodeError:
        continue
    method = request.get("method")
    request_id = request.get("id")
    params = request.get("params") or {}
    if method == "initialize":
        event("initialize")
        if INIT_MODE == "malformed":
            result(request_id, {"protocolVersion": 42})
        elif INIT_MODE == "wrong_version":
            result(request_id, {
                "protocolVersion": "1900-01-01",
                "capabilities": {},
                "serverInfo": {"name": "fixture", "version": "1"},
            })
        else:
            result(request_id, {
                "protocolVersion": params.get("protocolVersion", "2025-11-25"),
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "fixture", "version": "1"},
            })
    elif method == "notifications/initialized":
        event("initialized")
    elif method == "tools/list":
        cursor = params.get("cursor")
        event(f"list:{cursor or 'first'}")
        if cursor is None:
            result(request_id, {
                "tools": [tool("echo", "echo a label", {
                    "label": {"type": "string"},
                    "delay_ms": {"type": "integer"},
                    "mode": {"type": "string"},
                }), tool("fail", "return an MCP tool error")],
                "nextCursor": "page-2",
            })
        else:
            result(request_id, {"tools": [
                tool("delay_write", "write after a delay", {
                    "marker": {"type": "string"},
                    "delay_ms": {"type": "integer"},
                }),
                tool("oversize", "return a response over the configured bound"),
            ]})
    elif method == "tools/call":
        thread = threading.Thread(target=call_tool, args=(request_id, params), daemon=True)
        thread.start()
    elif method == "notifications/cancelled":
        cancelled_id = params.get("requestId")
        event(f"cancel_notification:{cancelled_id}")
        with CANCEL_LOCK:
            CANCELLED[json.dumps(cancelled_id, sort_keys=True)] = True
    elif request_id is not None:
        error(request_id, -32601, f"unsupported method {method}")

event("eof")
