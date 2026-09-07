"""Non-Rust model/v1 boundary fixture; all test behavior is export-configured."""
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))
from component_runtime import HostError, ProtocolError, run_component

settings = {}


def initialize(params):
    if params["protocol_version"] != "v3":
        raise ProtocolError("expected component v3")
    exports = []
    for export in params["exports"]:
        if (export["slot"], export["contract_version"], export["composition"]) != ("model", "v1", "select_one"):
            raise ProtocolError("expected model/v1 select_one")
        settings[export["module_id"]] = export["module_config"]
        if "pid_marker" in export["module_config"]:
            with Path(export["module_config"]["pid_marker"]).open("a") as file:
                file.write(str(os.getpid()) + "\n")
        exports.append({key: export[key] for key in ("slot", "module_id", "contract_version", "composition")})
        exports[-1]["module_features"] = []
    return {"protocol_version": "v3", "component_id": params["component_id"], "exports": exports}


def invoke(context, method, params):
    config = settings[context.export["module_id"]]
    if method == "describe":
        if params is not None:
            raise ProtocolError("describe expects null")
        return config["descriptor"]
    if method != "stream" or set(params) != {"request", "stream"}:
        raise ProtocolError("invalid model request")
    if "expected_input" in config and params != config["expected_input"]:
        raise ProtocolError("canonical model input changed")
    mode = config.get("mode", "normal")
    marker = Path(config["marker"]) if "marker" in config else None
    if marker:
        marker.write_text("started")
        context.on_cancel(lambda: marker.write_text("canceled"))
    if mode == "crash":
        os._exit(23)
    if mode == "forbidden":
        context.host_call("host.workflow.execute_tool", {})
    events = config.get("events", [])
    repeat = config.get("repeat", 1)
    count = 0
    for _ in range(repeat):
        for event in events:
            sequence = count + (1 if mode == "bad_sequence" else 0)
            try:
                context.host_call("host.model.emit", {"sequence": sequence, "event": event})
            except HostError:
                if mode != "bad_sequence":
                    if marker:
                        marker.write_text("consumer_closed")
                    raise
                break  # deliberately ignore rejection: host must still fail the stream
            count += 1
            if marker:
                marker.write_text(str(count))
    if mode == "wait":
        while not context.is_cancelled():
            time.sleep(0.005)
        context.ensure_active()
    terminal = config["terminal"]
    if mode == "bad_count":
        count += 1
    return {"event_count": count, "terminal": terminal}


run_component(initialize, invoke)
