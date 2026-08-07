#!/usr/bin/env python3
"""Dependency-free process SearchBackend example for Proteus.

The module speaks JSON-RPC 2.0 as one compact JSON object per stdout line and
uses ripgrep only as its search engine. Python is an example implementation
language, not part of the protocol.
"""

from __future__ import annotations

import json
import signal
import subprocess
import sys
from pathlib import PurePath
from typing import Any


PROTOCOL_VERSION = "v1"
SLOT = "search"
MODULE_ID = "python_rg"
CONTRACT_VERSION = "v1"

INITIALIZE_FIELDS = {
    "protocol_version",
    "slot",
    "module_id",
    "contract_version",
    "composition",
    "module_config",
    "host_features",
}
QUERY_FIELDS = {
    "text",
    "cwd",
    "max_results",
    "use_case",
    "starts_with",
    "ends_with",
}
REQUEST_FIELDS = {"jsonrpc", "id", "method", "params"}

active_rg: subprocess.Popen[str] | None = None


class ProtocolError(Exception):
    pass


def require_object(value: Any, fields: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ProtocolError(f"{label} must be an object")
    actual = set(value)
    if actual != fields:
        missing = sorted(fields - actual)
        unknown = sorted(actual - fields)
        raise ProtocolError(
            f"{label} fields mismatch: missing={missing}, unknown={unknown}"
        )
    return value


def require_string_list(value: Any, label: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ProtocolError(f"{label} must be an array of strings")
    return value


def validate_initialize(params: Any) -> None:
    params = require_object(params, INITIALIZE_FIELDS, "initialize params")
    expected_identity = (
        PROTOCOL_VERSION,
        SLOT,
        MODULE_ID,
        CONTRACT_VERSION,
        "select_one",
    )
    actual_identity = (
        params["protocol_version"],
        params["slot"],
        params["module_id"],
        params["contract_version"],
        params["composition"],
    )
    if actual_identity != expected_identity:
        raise ProtocolError(f"unsupported initialize params: {params!r}")
    host_features = require_string_list(
        params["host_features"], "initialize host_features"
    )
    if host_features:
        raise ProtocolError(f"unsupported host features: {host_features!r}")


def validate_query(params: Any) -> dict[str, Any]:
    query = require_object(params, QUERY_FIELDS, "SearchQuery")
    if not isinstance(query["text"], str):
        raise ProtocolError("SearchQuery.text must be a string")
    if not isinstance(query["cwd"], str):
        raise ProtocolError("SearchQuery.cwd must be a string")
    if (
        not isinstance(query["max_results"], int)
        or isinstance(query["max_results"], bool)
        or query["max_results"] < 0
    ):
        raise ProtocolError("SearchQuery.max_results must be a non-negative integer")
    if query["use_case"] is not None and not isinstance(query["use_case"], str):
        raise ProtocolError("SearchQuery.use_case must be a string or null")
    query["starts_with"] = require_string_list(
        query["starts_with"], "SearchQuery.starts_with"
    )
    query["ends_with"] = require_string_list(
        query["ends_with"], "SearchQuery.ends_with"
    )
    return query


def safe_search_roots(prefixes: list[str]) -> list[str]:
    roots: list[str] = []
    for prefix in prefixes:
        trimmed = prefix.strip().removeprefix("./").rstrip("/")
        if not trimmed or trimmed == ".":
            roots.append(".")
            continue
        path = PurePath(trimmed)
        if path.is_absolute() or ".." in path.parts:
            continue
        roots.append(str(path))
    return roots or ["."]


def path_matches(path: str, starts_with: list[str], ends_with: list[str]) -> bool:
    starts = not starts_with or any(path.startswith(prefix) for prefix in starts_with)
    ends = not ends_with or any(path.endswith(suffix) for suffix in ends_with)
    return starts and ends


def rg_text(value: Any, label: str) -> str:
    if not isinstance(value, dict) or not isinstance(value.get("text"), str):
        raise RuntimeError(f"ripgrep returned non-text {label}")
    return value["text"]


def stop_active_rg() -> None:
    global active_rg
    process = active_rg
    if process is None or process.poll() is not None:
        active_rg = None
        return
    process.terminate()
    try:
        process.wait(timeout=0.5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    active_rg = None


def search(query: dict[str, Any]) -> dict[str, Any]:
    global active_rg
    text = query["text"]
    max_results = query["max_results"]
    if not text.strip() or max_results == 0:
        return {"chunks": []}

    command = [
        "rg",
        "--json",
        "--max-columns",
        "2000",
        "--max-filesize",
        "1M",
        "--",
        text,
        *safe_search_roots(query["starts_with"]),
    ]
    chunks: list[dict[str, Any]] = []
    active_rg = subprocess.Popen(
        command,
        cwd=query["cwd"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    assert active_rg.stdout is not None
    reached_limit = False
    try:
        for line in active_rg.stdout:
            event = json.loads(line)
            if event.get("type") != "match":
                continue
            data = event.get("data")
            if not isinstance(data, dict):
                raise RuntimeError("ripgrep match event has no data object")
            path = rg_text(data.get("path"), "path").removeprefix("./")
            if not path_matches(path, query["starts_with"], query["ends_with"]):
                continue
            line_number = data.get("line_number")
            if not isinstance(line_number, int):
                raise RuntimeError("ripgrep match event has no integer line_number")
            content = rg_text(data.get("lines"), "lines").rstrip("\r\n")
            chunks.append(
                {
                    "source": f"process:{MODULE_ID}",
                    "path": path,
                    "content": content,
                    "score": None,
                    "metadata": {"line": line_number, "module_id": MODULE_ID},
                }
            )
            if len(chunks) >= max_results:
                reached_limit = True
                break
    except Exception:
        stop_active_rg()
        raise

    if reached_limit:
        stop_active_rg()
    else:
        status = active_rg.wait()
        active_rg = None
        if status not in (0, 1):
            raise RuntimeError(f"ripgrep exited with status {status}")
    return {"chunks": chunks}


def response(request_id: Any, result: Any) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error_response(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def handle_request(raw: Any, initialized: bool) -> tuple[dict[str, Any], bool]:
    request = require_object(raw, REQUEST_FIELDS, "JSON-RPC request")
    request_id = request["id"]
    if not isinstance(request_id, (int, str)) or isinstance(request_id, bool):
        raise ProtocolError("JSON-RPC id must be a string or integer")
    if request["jsonrpc"] != "2.0":
        raise ProtocolError("JSON-RPC version must be 2.0")
    method = request["method"]
    if not isinstance(method, str):
        raise ProtocolError("JSON-RPC method must be a string")

    if method == "initialize":
        if initialized:
            raise ProtocolError("initialize may be called only once")
        validate_initialize(request["params"])
        manifest = {
            "protocol_version": PROTOCOL_VERSION,
            "slot": SLOT,
            "module_id": MODULE_ID,
            "contract_version": CONTRACT_VERSION,
            "composition": "select_one",
            "module_features": [],
        }
        return response(request_id, manifest), True
    if not initialized:
        raise ProtocolError("module must be initialized before method calls")
    if method == "search":
        return response(request_id, search(validate_query(request["params"]))), initialized
    raise ProtocolError(f"unknown method: {method}")


def terminate_from_signal(_signum: int, _frame: Any) -> None:
    stop_active_rg()
    raise SystemExit(143)


def main() -> int:
    signal.signal(signal.SIGTERM, terminate_from_signal)
    signal.signal(signal.SIGINT, terminate_from_signal)
    initialized = False
    for line in sys.stdin:
        if not line.strip():
            continue
        request_id: Any = None
        try:
            raw = json.loads(line)
            if isinstance(raw, dict):
                request_id = raw.get("id")
            message, initialized = handle_request(raw, initialized)
        except ProtocolError as error:
            message = error_response(request_id, -32602, str(error))
        except Exception as error:  # explicit JSON-RPC failure, never stdout noise
            message = error_response(request_id, -32603, str(error))
        print(json.dumps(message, ensure_ascii=False, separators=(",", ":")), flush=True)
    stop_active_rg()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
