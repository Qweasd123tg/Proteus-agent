"""Tool exposure fixture with deliberately malformed selection modes."""
import copy
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))
from component_runtime import run_component, ProtocolError

settings = {}

def initialize(params):
    exports = []
    for export in params["exports"]:
        if (export["slot"], export["contract_version"], export["composition"]) != ("tool_exposure", "v3", "select_one"):
            raise ProtocolError("expected tool_exposure/v3")
        settings[export["module_id"]] = export["module_config"]
        item = {key: export[key] for key in ("slot", "module_id", "contract_version", "composition")}
        item["module_features"] = []
        exports.append(item)
    return {"protocol_version":"v3", "component_id":params["component_id"], "exports":exports}

def invoke(context, method, params):
    if method != "select": raise ProtocolError("select only")
    tools = copy.deepcopy(params["input"]["candidates"][:1])
    mode = settings[context.export["module_id"]]["mode"]
    if mode == "hosted":
        tools[0]["safety"] = "Network"
        tools[0]["surface"] = {"kind":"provider_hosted", "config":{"type":"web_search", "config":{
            "search_context_size":None,"allowed_domains":[],"blocked_domains":[],
            "external_web_access":True,"include_sources":False}}}
    elif mode == "schema": tools[0]["input_schema"] = {"type":"string"}
    elif mode == "parallel": tools[0]["supports_parallel_tool_calls"] = True
    elif mode == "invented": tools[0]["name"] = "unregistered_tool"
    elif mode == "duplicate": tools *= 2
    output = {"tools":tools, "metadata":None}
    if mode == "unknown": output["typo"] = True
    return {"result":output}

run_component(initialize, invoke)
