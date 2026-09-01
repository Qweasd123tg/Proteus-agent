#!/usr/bin/env python3
"""Scripted tool component for deterministic workflow journal evidence."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[5] / "examples" / "modules"))

from component_runtime import PROTOCOL_VERSION, ProtocolError, run_component  # noqa: E402


COMPONENT_ID = "project-check-tools"
MODULE_ID = "project-check-fixture-tools"
EXPORT = {
    "slot": "tool",
    "module_id": MODULE_ID,
    "contract_version": "v2",
    "composition": "ordered_many",
    "module_features": [],
}


def initialize(params):
    expected = {"protocol_version": PROTOCOL_VERSION, "component_id": COMPONENT_ID}
    if any(params.get(key) != value for key, value in expected.items()):
        raise ProtocolError("invalid project-check fixture initialization")
    exports = params.get("exports") or []
    actual = {
        (
            export.get("slot"),
            export.get("module_id"),
            export.get("contract_version"),
            export.get("composition"),
        )
        for export in exports
    }
    expected_exports = {("tool", MODULE_ID, "v2", "ordered_many")}
    if actual != expected_exports:
        raise ProtocolError(f"unexpected project-check exports: {exports!r}")
    return {
        "protocol_version": PROTOCOL_VERSION,
        "component_id": COMPONENT_ID,
        "exports": [EXPORT],
    }


def tool_spec(name, description, safety, properties=None, required=None):
    return {
        "name": name,
        "description": description,
        "input_schema": {
            "type": "object",
            "properties": properties or {},
            "required": required or [],
        },
        "surface": {"kind": "function", "strict": False, "output_schema": None},
        "safety": safety,
        "timeout_ms": 3000,
        "metadata": {"fixture": True},
    }


TOOLS = [
    tool_spec("git_status", "Scripted git status", "ReadOnly"),
    tool_spec(
        "list_dir",
        "Scripted root listing",
        "ReadOnly",
        {"path": {"type": "string"}},
    ),
    tool_spec(
        "shell",
        "Scripted test command",
        "RunsCommands",
        {
            "command": {"type": "string"},
            "timeout_ms": {"type": "integer"},
        },
        ["command"],
    ),
]


def result(call, output, metadata):
    return {
        "result": {
            "call_id": call.get("id"),
            "ok": True,
            "output": output,
            "content": [],
            "error": None,
            "metadata": metadata,
        }
    }


def invoke(context, method, params):
    if context.export != {"slot": "tool", "module_id": MODULE_ID}:
        raise ProtocolError("unexpected project-check export")
    if method == "list":
        return {"result": TOOLS}
    if method != "invoke":
        raise ProtocolError(f"unexpected project-check method: {method}")

    call = params.get("call") or {}
    name = call.get("name")
    if name == "git_status":
        return result(call, "## main", {"fixture": True})
    if name == "list_dir":
        if (call.get("args") or {}).get("path") != ".":
            raise ProtocolError("project-check list_dir must inspect the root")
        return result(call, "file\tCargo.toml\ndir\tsrc", {"fixture": True})
    if name == "shell":
        args = call.get("args") or {}
        if args.get("command") != "cargo test":
            raise ProtocolError(f"unexpected project-check command: {args!r}")
        return result(
            call,
            "test result: ok. 1 passed; 0 failed",
            {"fixture": True, "exit_code": 0, "timed_out": False},
        )
    raise ProtocolError(f"unexpected project-check tool: {name!r}")


run_component(initialize, invoke)
