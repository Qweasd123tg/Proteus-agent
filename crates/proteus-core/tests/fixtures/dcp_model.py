"""Scripted model for real DCP hook/tool/journal evidence; no runtime heuristics."""
import json
import re
import sys
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))
from component_runtime import run_component

settings = {}
count = 0


def initialize(params):
    settings.update(params["exports"][0]["module_config"])
    return dict(protocol_version="v3", component_id=params["component_id"], exports=[dict(
        slot="model", module_id="dcp-probe", contract_version="v12", composition="select_one", module_features=[], config_schema=None)])


def invoke(context, method, params):
    global count
    if method == "describe":
        return settings["descriptor"]
    if method in ("catalog", "quota"):
        return None
    assert method == "stream"
    count += 1
    request = params["request"]
    with Path(settings["capture"]).open("a") as file:
        file.write(json.dumps(request) + "\n")
    if count == 1:
        text = "Finished obsolete research. " * 100
        call = None
    elif count == 2:
        refs = []
        for msg in request["messages"]:
            matches = re.findall(r"@[1-9]\d*@", "\n".join(p["payload"]["Text"]["text"] for p in msg["parts"] if "Text" in p["payload"]))
            if matches:
                refs.append(matches[-1])
        assert len(refs) >= 2, [(m["role"], m["parts"]) for m in request["messages"]]
        call = dict(id=str(uuid.uuid4()), name="compress", surface="function", raw_arguments=None,
                    args=dict(topic="finished research", content=[dict(startId=refs[0], endId=refs[1], summary="Research complete.")]))
    elif settings.get("fail_after_compress"):
        return dict(event_count=0, terminal=dict(kind="failure", failure=dict(kind="protocol", message="scripted failure", completed_messages=[])))
    else:
        text = "done"
        call = None
    payload = {"ToolCall": {"call": call}} if call else {"Text": {"text": text}}
    message = dict(id=str(uuid.uuid4()), role="Assistant", phase=None, name=None, tool_call_id=None, metadata=None,
                   parts=[dict(part_id=str(uuid.uuid4()), provenance="model", scope="conversation", payload=payload)])
    response = dict(messages=[message], tool_calls=[call] if call else [], finish_reason="ToolCalls" if call else "Stop",
                    usage=None, end_turn=None, provider_metadata=None)
    return dict(event_count=0, terminal=dict(kind="response", response=response))


run_component(initialize, invoke)
