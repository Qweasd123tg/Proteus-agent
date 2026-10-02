"""Exercise workflow/v19 using execution capabilities without conversation state."""
import json
import sys
import uuid
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))
from component_runtime import HostError, ProtocolError, run_component


def initialize(params):
    exports = []
    for export in params["exports"]:
        if (export["slot"], export["contract_version"], export["composition"]) != ("workflow", "v19", "select_one"):
            raise ProtocolError("expected workflow/v19")
        exports.append({key: export[key] for key in ("slot", "module_id", "contract_version", "composition")})
        exports[-1]["module_features"] = []
    return {"protocol_version": "v3", "component_id": params["component_id"], "exports": exports}


def invoke(context, method, params):
    runtime = params["runtime"]
    uuid.UUID(runtime["execution_id"])
    if method != "run" or runtime["conversation"] is not None or runtime["model_ref"] is not None or params["history"]:
        raise ProtocolError("expected standalone model-free invocation")
    status = context.host_call("host.runtime.status", {})
    if status != {"cancelled": False, "queued_user_messages": 0}:
        raise ProtocolError("unexpected runtime status")
    if context.host_call("host.tools.visible", {"cwd": params["task"]["cwd"]}):
        raise ProtocolError("unexpected tools")
    context.host_call("host.context.build", {"task": params["task"]})
    try:
        context.host_call("host.model.complete", {"request": json.loads(params["task"]["text"])})
    except HostError as error:
        if "no model is configured" not in str(error):
            raise ProtocolError(f"unexpected missing-model error: {error}")
    else:
        raise ProtocolError("an absent model must not complete a request")
    output = {"text": "standalone completed", "metadata": {"execution_id": runtime["execution_id"]}}
    return {"status": "success", "result": {"output": output, "new_messages": [], "history_replacement": None, "compactions": []}}

run_component(initialize, invoke)
