#!/usr/bin/env python3
"""Barrier-controlled tool component for streamed execution/replay evidence."""
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[5] / "examples/modules"))
from component_runtime import PROTOCOL_VERSION, ProtocolError, run_component

EXPORT = {
    "slot": "tool", "module_id": "stream-tools", "contract_version": "v3",
    "composition": "ordered_many", "module_features": [],
}
LOCK = threading.Lock()
ACTIVE = set()


def initialize(params):
    if (params.get("protocol_version") != PROTOCOL_VERSION
            or params.get("component_id") != "stream-tools-component"):
        raise ProtocolError("invalid stream tools initialization")
    return {"protocol_version": PROTOCOL_VERSION,
            "component_id": params["component_id"], "exports": [EXPORT]}


def spec(name, safety, parallel):
    return {
        "name": name, "description": "Barrier-controlled stream probe",
        "input_schema": {"type": "object", "properties": {
            "directory": {"type": "string"}, "label": {"type": "string"}},
            "required": ["directory", "label"], "additionalProperties": False},
        "surface": {"kind": "function", "strict": False, "output_schema": None},
        "safety": safety, "supports_parallel_tool_calls": parallel, "timeout_ms": 10000, "metadata": {},
    }


def invoke(context, method, params):
    if context.export != {"slot": "tool", "module_id": "stream-tools"}:
        raise ProtocolError("unexpected export")
    if method == "list":
        return {"result": [spec("parallel_probe", "RunsCommands", True),
                           spec("exclusive_probe", "WritesFiles", False),
                           spec("serial_read_probe", "ReadOnly", False)]}
    if method != "invoke":
        raise ProtocolError("unexpected method")
    call = params["call"]
    label = call["args"]["label"]
    directory = Path(call["args"]["directory"])
    exclusive = call["name"] != "parallel_probe"
    context.on_cancel(lambda: (directory / f"canceled-{label}").write_text("canceled"))
    with LOCK:
        if (exclusive and ACTIVE) or "exclusive" in ACTIVE:
            raise ProtocolError("exclusive call overlapped another tool")
        ACTIVE.add("exclusive" if exclusive else label)
    try:
        # Markers synchronize the fixture only; the exclusive tool's effects.log
        # is the actual effect whose approval and replay behavior are checked.
        (directory / f"started-{label}").write_text("started")
        deadline = time.monotonic() + 10
        while not (directory / f"release-{label}").exists():
            context.ensure_active()
            if time.monotonic() > deadline:
                raise ProtocolError(f"fixture barrier timed out: {label}")
            time.sleep(0.005)
        context.ensure_active()
        if call["name"] == "exclusive_probe":
            with (directory / "effects.log").open("a") as output:
                output.write(label + "\n")
        return {"result": {"call_id": call["id"], "ok": True, "output": label,
                           "content": [], "error": None, "metadata": {}}}
    finally:
        with LOCK:
            ACTIVE.remove("exclusive" if exclusive else label)


run_component(initialize, invoke)
