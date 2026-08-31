#!/usr/bin/env python3
"""Concurrent memory/v2 component for Phase 8B admission tests."""

from __future__ import annotations

import json
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))

from component_runtime import PROTOCOL_VERSION, ProtocolError, run_component  # noqa: E402


COMPONENT_ID = "phase8-memory-component"
MODULE_ID = "phase8-memory"
EXPORT = {
    "slot": "memory",
    "module_id": MODULE_ID,
    "contract_version": "v2",
    "composition": "select_one",
    "module_features": [],
}
record_path: Path | None = None
record_lock = threading.Lock()


def initialize(params):
    global record_path
    exports = params.get("exports")
    if params.get("protocol_version") != PROTOCOL_VERSION or params.get("component_id") != COMPONENT_ID:
        raise ProtocolError("invalid Phase 8B component identity")
    if not isinstance(exports, list) or len(exports) != 1:
        raise ProtocolError("Phase 8B memory component requires one export")
    export = exports[0]
    identity = (
        export.get("slot"),
        export.get("module_id"),
        export.get("contract_version"),
        export.get("composition"),
    )
    if identity != ("memory", MODULE_ID, "v2", "select_one"):
        raise ProtocolError(f"invalid Phase 8B memory export: {identity!r}")
    config = export.get("module_config")
    if not isinstance(config, dict) or not isinstance(config.get("record_path"), str):
        raise ProtocolError("Phase 8B memory record_path is required")
    record_path = Path(config["record_path"])
    return {
        "protocol_version": PROTOCOL_VERSION,
        "component_id": params["component_id"],
        "exports": [EXPORT],
    }


def append_record(value):
    if record_path is None:
        raise ProtocolError("memory component was not initialized")
    record_path.parent.mkdir(parents=True, exist_ok=True)
    with record_lock:
        with record_path.open("a", encoding="utf-8") as output:
            output.write(json.dumps(value, ensure_ascii=False) + "\n")


def validate_attribution(params):
    attribution = params.get("attribution")
    if not isinstance(attribution, dict) or set(attribution) != {"execution_id", "agent"}:
        raise ProtocolError("memory/v2 attribution is mandatory and strict")
    if not isinstance(attribution.get("execution_id"), str) or attribution.get("agent") is not None:
        raise ProtocolError("Phase 8B top-level memory requires detached attribution")
    return attribution


def invoke(context, method, params):
    if context.export != {"slot": "memory", "module_id": MODULE_ID}:
        raise ProtocolError("unexpected Phase 8B export")
    attribution = validate_attribution(params)
    if method == "recall":
        append_record({"method": method, "attribution": attribution, "query": params.get("query")})
        return {"result": []}
    if method != "remember":
        raise ProtocolError(f"unexpected memory method: {method}")

    item = params.get("item")
    if not isinstance(item, dict):
        raise ProtocolError("remember item must be an object")
    metadata = item.get("metadata")
    metadata = metadata if isinstance(metadata, dict) else {}
    start_marker = metadata.get("start_marker")
    if isinstance(start_marker, str):
        Path(start_marker).write_text("started\n", encoding="utf-8")
    cancel_marker = metadata.get("cancel_marker")
    if isinstance(cancel_marker, str):
        context.on_cancel(lambda: Path(cancel_marker).write_text("canceled\n", encoding="utf-8"))

    if metadata.get("wait_for_cancel") is True:
        while True:
            context.ensure_active()
            time.sleep(0.01)
    deadline = time.monotonic() + max(metadata.get("delay_ms", 0), 0) / 1000
    while time.monotonic() < deadline:
        context.ensure_active()
        time.sleep(0.01)

    append_record({"method": method, "attribution": attribution, "item": item})
    return {"result": None}


run_component(initialize, invoke)
