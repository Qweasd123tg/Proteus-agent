#!/usr/bin/env python3
"""Minimal process policy used by real-peer integration tests."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "examples/modules"))

from component_runtime import PROTOCOL_VERSION, ProtocolError, run_component  # noqa: E402

SLOT = "policy"
MODULE_ID = "fixture_allow_all"


def initialize(params):
    exports = params.get("exports")
    if (
        params.get("protocol_version") != PROTOCOL_VERSION
        or not isinstance(exports, list)
        or len(exports) != 1
    ):
        raise ProtocolError("invalid policy fixture initialization")
    export = exports[0]
    identity = (
        export.get("slot"),
        export.get("module_id"),
        export.get("contract_version"),
        export.get("composition"),
    )
    if identity != (SLOT, MODULE_ID, "v1", "select_one"):
        raise ProtocolError("unexpected policy fixture export")
    return {
        "protocol_version": PROTOCOL_VERSION,
        "component_id": params["component_id"],
        "exports": [
            {
                "slot": SLOT,
                "module_id": MODULE_ID,
                "contract_version": "v1",
                "composition": "select_one",
                "module_features": [],
            }
        ],
    }


def invoke(context, method, _params):
    if context.export != {"slot": SLOT, "module_id": MODULE_ID}:
        raise ProtocolError("unexpected policy fixture target")
    if method not in {"evaluate", "evaluate_visibility"}:
        raise ProtocolError("unexpected policy fixture method")
    return {"result": "Allow"}


run_component(initialize, invoke)
