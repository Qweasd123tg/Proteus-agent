#!/usr/bin/env python3
"""User-only tool commands use the ordinary process policy and host surface."""
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))
from component_runtime import run_component, ProtocolError


def initialize(params):
    exports = []
    for item in params["exports"]:
        expected = {"tool": ("probe.commands", "v5", "ordered_many"),
                    "policy": ("probe.policy", "v2", "select_one")}.get(item["slot"])
        if expected != (item["module_id"], item["contract_version"], item["composition"]):
            raise ProtocolError("unexpected command fixture binding")
        exports.append({key: item[key] for key in ("slot", "module_id", "contract_version", "composition")}
                       | {"module_features": [], "config_schema": None})
    return {"protocol_version": "v3", "component_id": params["component_id"], "exports": exports}


def invoke(context, method, params):
    if context.export["slot"] == "policy":
        return {"result": "Allow" if method == "evaluate_visibility" else {"Ask": {"reason": "command approval"}}}
    if method == "list":
        return {"result": [{"spec": {
            "name": "probe_command", "description": "User command fixture",
            "input_schema": {"type": "object", "properties": {"arguments": {"type": "string"}},
                             "required": ["arguments"], "additionalProperties": False},
            "surface": {"kind": "function", "strict": False, "output_schema": None},
            "safety": "ReadOnly", "supports_parallel_tool_calls": False,
            "timeout_ms": 10000, "metadata": {}},
            "model_visible": False,
            "user_command": {"name": "probe", "description": "Command fixture", "arguments": "[wait]"}}]}
    if method != "invoke":
        raise ProtocolError("unexpected command method")
    if params["attribution"]["agent"] is not None:
        raise ProtocolError("user command unexpectedly opened an agent turn")
    snapshot = context.host_call("host.conversation.snapshot", {})
    Path(params["cwd"], "command-started").write_text("started")
    if params["call"]["args"]["arguments"] == "wait":
        while True:
            context.ensure_active()
            time.sleep(0.01)
    return {"result": {"call_id": params["call"]["id"], "ok": True,
                       "output": json.dumps(snapshot), "content": [], "error": None, "metadata": {}}}


run_component(initialize, invoke)
