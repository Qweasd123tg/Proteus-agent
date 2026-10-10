#!/usr/bin/env python3
"""Standalone tool/search implementation using tgrep, Component v3 / tool v5."""

from __future__ import annotations

import signal
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from component_runtime import (  # noqa: E402
    PROTOCOL_VERSION, InvocationContext, ProtocolError, require_object, run_component,
)
from backend import search, stop_all, validate_args  # noqa: E402

MODULE_ID = "tgrep_search"
EXPORT = {"slot": "tool", "module_id": MODULE_ID}
config: dict[str, str] = {}


def initialize(params: Any) -> dict[str, Any]:
    global config
    params = require_object(params, {"protocol_version", "component_id", "exports"}, "initialize")
    if params["protocol_version"] != PROTOCOL_VERSION:
        raise ProtocolError("unsupported component protocol")
    if not isinstance(params["component_id"], str) or not params["component_id"].strip():
        raise ProtocolError("component_id must be a non-empty string")
    if not isinstance(params["exports"], list) or len(params["exports"]) != 1:
        raise ProtocolError("tgrep component requires exactly one tool export")
    export = require_object(params["exports"][0], {
        "slot", "module_id", "contract_version", "composition", "module_config", "host_features",
    }, "initialize export")
    if (export["slot"] != "tool" or export["module_id"] != MODULE_ID
            or export["contract_version"] != "v5" or export["composition"] != "ordered_many"):
        raise ProtocolError("expected tool/tgrep_search v5 ordered_many")
    if export["host_features"] != []:
        raise ProtocolError("tgrep tool does not use host features")
    supplied = export["module_config"]
    if not isinstance(supplied, dict) or set(supplied) - {"binary", "index_path"}:
        raise ProtocolError("tgrep config accepts only binary and index_path")
    if any(not isinstance(value, str) or not value.strip() for value in supplied.values()):
        raise ProtocolError("binary and index_path must be non-empty strings")
    config = supplied.copy()
    return {"protocol_version": PROTOCOL_VERSION, "component_id": params["component_id"],
            "exports": [{**EXPORT, "contract_version": "v5", "composition": "ordered_many",
                         "module_features": [], "config_schema": None}]}


def invoke(context: InvocationContext, method: str, params: Any) -> dict[str, Any]:
    if context.export != EXPORT:
        raise ProtocolError("unknown tgrep export")
    if method == "list":
        return {"result": [{"spec": {
            "name": "search",
            "description": "Search workspace text by regex. Indexed searches may lag edits; use freshness=current to check recent changes.",
            "input_schema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "max_results": {"type": "integer", "minimum": 0},
                "use_case": {"type": "string"},
                "starts_with": {"type": "array", "items": {"type": "string"}},
                "ends_with": {"type": "array", "items": {"type": "string"}},
                "freshness": {"type": "string", "enum": ["indexed", "current"],
                              "default": "indexed"},
            }, "required": ["query"], "additionalProperties": False},
            "surface": {"kind": "function", "strict": False, "output_schema": None},
            "safety": "ReadOnly", "supports_parallel_tool_calls": True,
            "timeout_ms": 65000, "metadata": {"hot": True, "category": "search"},
        }, "model_visible": True, "user_command": None}]}
    if method != "invoke":
        raise ProtocolError(f"unknown tool method: {method}")
    request = require_object(params, {"call", "cwd", "attribution", "skills"}, "tool request")
    require_object(request["attribution"], {"execution_id", "agent"}, "attribution")
    call = require_object(request["call"], {"id", "name", "args", "surface", "raw_arguments"}, "ToolCall")
    if call["name"] != "search" or call["surface"] != "function":
        raise ProtocolError("expected search function call")
    if not isinstance(request["cwd"], str):
        raise ProtocolError("cwd must be a string")
    args = validate_args(call["args"])
    chunks, limit_reached = search(args, request["cwd"], config, context)
    output = "\n".join(f"{chunk['path']}:{chunk['metadata']['line']}: {chunk['content'].strip()}"
                       for chunk in chunks)
    return {"result": {"call_id": call["id"], "ok": True,
                       "output": output or "(no matches)", "content": [], "error": None,
                       "metadata": {"results": len(chunks), "chunks": chunks,
                                    "freshness": args["freshness"], "limit_reached": limit_reached}}}


def terminate(_signum: int, _frame: Any) -> None:
    stop_all()
    raise SystemExit(143)


def main() -> int:
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    try:
        return run_component(initialize, invoke)
    finally:
        stop_all()


if __name__ == "__main__":
    raise SystemExit(main())
