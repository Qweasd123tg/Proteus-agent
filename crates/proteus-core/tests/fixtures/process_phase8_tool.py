#!/usr/bin/env python3
"""Concurrent tool+policy component for Phase 8A runtime admission tests."""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))

from component_runtime import PROTOCOL_VERSION, ProtocolError, run_component  # noqa: E402


COMPONENT_ID = "phase8-execution-component"
TOOL_MODULE_ID = "phase8-tools"
POLICY_MODULE_ID = "phase8-allow-all"
TOOL_EXPORT = {
    "slot": "tool",
    "module_id": TOOL_MODULE_ID,
    "contract_version": "v2",
    "composition": "ordered_many",
    "module_features": [],
}
POLICY_EXPORT = {
    "slot": "policy",
    "module_id": POLICY_MODULE_ID,
    "contract_version": "v1",
    "composition": "select_one",
    "module_features": [],
}


def initialize(params):
    exports = params.get("exports")
    expected = {
        ("tool", TOOL_MODULE_ID, "v2", "ordered_many"),
        ("policy", POLICY_MODULE_ID, "v1", "select_one"),
    }
    actual = {
        (
            export.get("slot"),
            export.get("module_id"),
            export.get("contract_version"),
            export.get("composition"),
        )
        for export in exports or []
    }
    if (
        params.get("protocol_version") != PROTOCOL_VERSION
        or params.get("component_id") != COMPONENT_ID
        or actual != expected
    ):
        raise ProtocolError("invalid Phase 8 component initialization")
    return {
        "protocol_version": PROTOCOL_VERSION,
        "component_id": params["component_id"],
        "exports": [TOOL_EXPORT, POLICY_EXPORT],
    }


def tool_spec():
    return {
        "name": "phase8_probe",
        "description": "Phase 8 top-level execution probe",
        "input_schema": {
            "type": "object",
            "properties": {
                "label": {"type": "string"},
                "delay_ms": {"type": "integer", "minimum": 0},
                "wait_for_cancel": {"type": "boolean"},
                "start_marker": {"type": "string"},
                "cancel_marker": {"type": "string"},
            },
            "additionalProperties": False,
        },
        "surface": {"kind": "function", "strict": False, "output_schema": None},
        "safety": "ReadOnly",
        "timeout_ms": 750,
        "metadata": {"fixture": True},
    }


def invoke_tool(context, method, params):
    if method == "list":
        return {"result": [tool_spec()]}
    if method != "invoke":
        raise ProtocolError(f"unexpected tool method: {method}")

    call = params.get("call") or {}
    attribution = params.get("attribution") or {}
    if attribution.get("agent") is not None:
        raise ProtocolError("Phase 8 invocation unexpectedly carried agent attribution")
    args = call.get("args") or {}
    start_marker = args.get("start_marker")
    if start_marker:
        Path(start_marker).write_text("started\n", encoding="utf-8")
    marker = args.get("cancel_marker")
    if marker:
        context.on_cancel(lambda: Path(marker).write_text("canceled\n", encoding="utf-8"))

    if args.get("wait_for_cancel", False):
        while True:
            context.ensure_active()
            time.sleep(0.01)
    else:
        deadline = time.monotonic() + max(args.get("delay_ms", 0), 0) / 1000
        while time.monotonic() < deadline:
            context.ensure_active()
            time.sleep(0.01)

    label = args.get("label", "phase8")
    return {
        "result": {
            "call_id": call.get("id"),
            "ok": True,
            "output": f"phase8:{label}",
            "content": [],
            "error": None,
            "metadata": {
                "label": label,
                "saw_detached_attribution": True,
            },
        }
    }


def invoke(context, method, params):
    if context.export == {"slot": "tool", "module_id": TOOL_MODULE_ID}:
        return invoke_tool(context, method, params)
    if context.export == {"slot": "policy", "module_id": POLICY_MODULE_ID}:
        if method not in {"evaluate", "evaluate_visibility"}:
            raise ProtocolError(f"unexpected policy method: {method}")
        return {"result": "Allow"}
    raise ProtocolError("unexpected Phase 8 component export")


run_component(initialize, invoke)
