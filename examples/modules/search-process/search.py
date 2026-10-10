#!/usr/bin/env python3
"""Dependency-free search tool and context provider example for Proteus.

The module speaks JSON-RPC 2.0 as one compact JSON object per stdout line and
uses ripgrep only as its search engine. Python is an example implementation
language, not part of the protocol.
"""

from __future__ import annotations

import json
import signal
import subprocess
import sys
import threading
from pathlib import Path, PurePath
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from component_runtime import (  # noqa: E402
    PROTOCOL_VERSION,
    InvocationContext,
    ProtocolError,
    require_object,
    run_component,
)

MODULE_ID = "python_rg"
CONTRACTS = {"tool": "v5", "context_provider": "v4"}
active_exports: list[dict[str, Any]] = []

INITIALIZE_FIELDS = {
    "protocol_version",
    "component_id",
    "exports",
}
EXPORT_INITIALIZE_FIELDS = {
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

active_rg: set[subprocess.Popen[str]] = set()
active_rg_lock = threading.Lock()


def require_string_list(value: Any, label: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ProtocolError(f"{label} must be an array of strings")
    return value


def validate_initialize(params: Any) -> str:
    global active_exports
    params = require_object(params, INITIALIZE_FIELDS, "initialize params")
    if params["protocol_version"] != PROTOCOL_VERSION:
        raise ProtocolError(f"unsupported component protocol: {params['protocol_version']!r}")
    component_id = params["component_id"]
    if not isinstance(component_id, str) or not component_id.strip():
        raise ProtocolError("initialize component_id must be a non-empty string")
    exports = params["exports"]
    if not isinstance(exports, list) or not exports:
        raise ProtocolError("python search component requires explicit exports")
    admitted = []
    for value in exports:
        export = require_object(value, EXPORT_INITIALIZE_FIELDS, "initialize export")
        slot = export["slot"]
        if (slot not in CONTRACTS or export["module_id"] != MODULE_ID
                or export["contract_version"] != CONTRACTS[slot]
                or export["composition"] != "ordered_many"):
            raise ProtocolError(f"unsupported initialize export: {export!r}")
        if any(entry["slot"] == slot for entry in admitted):
            raise ProtocolError(f"duplicate export: {slot}")
        if not isinstance(export["module_config"], dict):
            raise ProtocolError("initialize module_config must be an object")
        if require_string_list(export["host_features"], "initialize host_features"):
            raise ProtocolError("search exports do not use host features")
        admitted.append({"slot": slot, "module_id": MODULE_ID,
                         "contract_version": CONTRACTS[slot], "composition": "ordered_many",
                         "module_features": [], "config_schema": None})
    active_exports = admitted
    return component_id


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


def normalize_prefix(prefix: str) -> str:
    while prefix.startswith("./"):
        prefix = prefix[2:]
    return prefix


def safe_search_roots(prefixes: list[str], cwd: str) -> list[str]:
    roots: list[str] = []
    for prefix in prefixes:
        trimmed = normalize_prefix(prefix).rstrip("/")
        if not trimmed or trimmed == ".":
            roots.append(".")
            continue
        path = PurePath(trimmed)
        if path.is_absolute() or ".." in path.parts:
            continue
        # A literal prefix can include several siblings and need not exist.
        # Only a trailing slash allows narrowing directly to that directory.
        root = path if prefix.endswith("/") else path.parent
        while root != PurePath(".") and not (Path(cwd) / root).is_dir():
            root = root.parent
        roots.append(str(root))
    if not roots or "." in roots:
        return ["."]
    unique_roots = sorted(set(roots))
    return [
        root for root in unique_roots
        if not any(PurePath(parent) in PurePath(root).parents for parent in unique_roots)
    ]


def path_matches(path: str, starts_with: list[str], ends_with: list[str]) -> bool:
    starts = not starts_with or any(path.startswith(normalize_prefix(prefix)) for prefix in starts_with)
    ends = not ends_with or any(path.endswith(suffix) for suffix in ends_with)
    return starts and ends


def rg_text(value: Any, label: str) -> str:
    if not isinstance(value, dict) or not isinstance(value.get("text"), str):
        raise RuntimeError(f"ripgrep returned non-text {label}")
    return value["text"]


def stop_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=0.5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    with active_rg_lock:
        active_rg.discard(process)


def search(query: dict[str, Any], context: InvocationContext) -> list[dict[str, Any]]:
    text = query["text"]
    max_results = query["max_results"]
    if not text.strip() or max_results == 0:
        return []

    command = [
        "rg",
        "--json",
        "--max-columns",
        "2000",
        "--max-filesize",
        "1M",
        "--",
        text,
        *safe_search_roots(query["starts_with"], query["cwd"]),
    ]
    chunks: list[dict[str, Any]] = []
    process = subprocess.Popen(
        command,
        cwd=query["cwd"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    with active_rg_lock:
        active_rg.add(process)
    context.on_cancel(lambda: stop_process(process))
    assert process.stdout is not None
    reached_limit = False
    try:
        for line in process.stdout:
            context.ensure_active()
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
                    "render_mode": "source_annotated",
                    "score": None,
                    "metadata": {"line": line_number, "module_id": MODULE_ID},
                }
            )
            if len(chunks) >= max_results:
                reached_limit = True
                break
    except Exception:
        stop_process(process)
        raise

    if reached_limit:
        stop_process(process)
    else:
        status = process.wait()
        with active_rg_lock:
            active_rg.discard(process)
        context.ensure_active()
        if status not in (0, 1):
            raise RuntimeError(f"ripgrep exited with status {status}")
    return chunks


def initialize(params: Any) -> dict[str, Any]:
    component_id = validate_initialize(params)
    return {
        "protocol_version": PROTOCOL_VERSION,
        "component_id": component_id,
        "exports": active_exports,
    }


def invoke(context: InvocationContext, method: str, params: Any) -> dict[str, Any]:
    slot = context.export.get("slot")
    if not any(context.export == {"slot": entry["slot"], "module_id": MODULE_ID}
               for entry in active_exports):
        raise ProtocolError(f"unknown component export: {context.export!r}")
    if slot == "context_provider":
        if method == "catalog":
            return {"result": None}
        if method != "provide":
            raise ProtocolError(f"unknown provider method: {method}")
        request = require_object(params, {"input", "attribution", "skills"}, "provider request")
        require_object(request["attribution"], {"execution_id", "agent"}, "attribution")
        source = require_object(request["input"], {"provider_id", "task", "metadata"}, "provider input")
        if source["provider_id"] != MODULE_ID:
            raise ProtocolError("provider_id must select python_rg")
        return {"result": search(validate_query(source["metadata"]), context)}
    if method == "list":
        return {"result": [{"spec": {
            "name": "search", "description": "Search workspace text with ripgrep",
            "input_schema": {"type": "object", "properties": {
                "query": {"type": "string"}, "max_results": {"type": "integer", "minimum": 0},
                "use_case": {"type": "string"},
                "starts_with": {"type": "array", "items": {"type": "string"}},
                "ends_with": {"type": "array", "items": {"type": "string"}},
            }, "required": ["query"]},
            "surface": {"kind": "function", "strict": False, "output_schema": None},
            "safety": "ReadOnly", "supports_parallel_tool_calls": True,
            "timeout_ms": 60000, "metadata": {"hot": True, "category": "search"},
        }, "model_visible": True, "user_command": None}]}
    if method != "invoke":
        raise ProtocolError(f"unknown tool method: {method}")
    request = require_object(params, {"call", "cwd", "attribution", "skills"}, "tool request")
    require_object(request["attribution"], {"execution_id", "agent"}, "attribution")
    call = require_object(request["call"], {"id", "name", "args", "surface", "raw_arguments"}, "ToolCall")
    if call["name"] != "search" or call["surface"] != "function" or not isinstance(call["args"], dict):
        raise ProtocolError("expected search function call")
    args = call["args"]
    query = validate_query({"text": args.get("query"), "cwd": request["cwd"],
                            "max_results": args.get("max_results", 20), "use_case": args.get("use_case"),
                            "starts_with": args.get("starts_with", []), "ends_with": args.get("ends_with", [])})
    chunks = search(query, context)
    output = "\n".join(f"{chunk['path']}:{chunk['metadata']['line']}: {chunk['content'].strip()}" for chunk in chunks)
    return {"result": {"call_id": call["id"], "ok": True, "output": output or "(no matches)",
                       "content": [], "error": None, "metadata": {"results": len(chunks), "chunks": chunks}}}


def terminate_from_signal(_signum: int, _frame: Any) -> None:
    with active_rg_lock:
        processes = list(active_rg)
    for process in processes:
        stop_process(process)
    raise SystemExit(143)


def main() -> int:
    signal.signal(signal.SIGTERM, terminate_from_signal)
    signal.signal(signal.SIGINT, terminate_from_signal)
    run_component(initialize, invoke)


if __name__ == "__main__":
    raise SystemExit(main())
