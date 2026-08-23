#!/usr/bin/env python3
"""Dependency-free process HistoryCompactor example for Proteus.

The module keeps a valid suffix beginning at one of the most recent user
turns. Contract v1 permits the same host.model.complete callback for every
compactor, but this deterministic example does not need to call it.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from component_runtime import (  # noqa: E402
    PROTOCOL_VERSION,
    InvocationContext,
    ProtocolError,
    require_object,
    run_component,
)

SLOT = "compactor"
MODULE_ID = "python_suffix"
CONTRACT_VERSION = "v1"

INITIALIZE_FIELDS = {
    "protocol_version",
    "component_id",
    "exports",
}
EXPORT_INITIALIZE_FIELDS = {
    "slot",
    "module_id",
    "contract_version",
    "composition",
    "module_config",
    "host_features",
}
INPUT_FIELDS = {
    "task",
    "model_ref",
    "messages",
    "token_estimate",
    "window_tokens",
    "config",
    "reason",
}
STRATEGY_FIELDS = {"trigger_messages", "retain_user_turns"}


def positive_int(value: Any, label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
        raise ProtocolError(f"{label} must be a positive integer")
    return value


def validate_initialize(params: Any) -> str:
    params = require_object(params, INITIALIZE_FIELDS, "initialize params")
    if params["protocol_version"] != PROTOCOL_VERSION:
        raise ProtocolError(f"unsupported component protocol: {params['protocol_version']!r}")
    component_id = params["component_id"]
    if not isinstance(component_id, str) or not component_id.strip():
        raise ProtocolError("initialize component_id must be a non-empty string")
    exports = params["exports"]
    if not isinstance(exports, list) or len(exports) != 1:
        raise ProtocolError("python compactor component requires exactly one export")
    export = require_object(
        exports[0], EXPORT_INITIALIZE_FIELDS, "initialize export"
    )
    expected_identity = (
        SLOT,
        MODULE_ID,
        CONTRACT_VERSION,
        "select_one",
    )
    actual_identity = (
        export["slot"],
        export["module_id"],
        export["contract_version"],
        export["composition"],
    )
    if actual_identity != expected_identity:
        raise ProtocolError(f"unsupported initialize export: {export!r}")
    if not isinstance(export["module_config"], dict):
        raise ProtocolError("initialize module_config must be an object")
    if export["host_features"] != []:
        raise ProtocolError("compactor v1 does not negotiate host features")
    return component_id


def validate_input(params: Any) -> tuple[dict[str, Any], int, int]:
    compaction = require_object(params, INPUT_FIELDS, "CompactionInput")
    if not isinstance(compaction["messages"], list):
        raise ProtocolError("CompactionInput.messages must be an array")
    if any(not isinstance(message, dict) for message in compaction["messages"]):
        raise ProtocolError("CompactionInput.messages must contain objects")

    strategy = compaction["config"]
    if strategy is None:
        strategy = {}
    if not isinstance(strategy, dict):
        raise ProtocolError("CompactionInput.config must be an object or null")
    unknown = sorted(set(strategy) - STRATEGY_FIELDS)
    if unknown:
        raise ProtocolError(f"unknown compactor strategy fields: {unknown}")
    trigger_messages = positive_int(strategy.get("trigger_messages", 12), "trigger_messages")
    retain_user_turns = positive_int(
        strategy.get("retain_user_turns", 2), "retain_user_turns"
    )
    return compaction, trigger_messages, retain_user_turns


def is_context_message(message: dict[str, Any]) -> bool:
    return message.get("name") == "context"


def is_user_message(message: dict[str, Any]) -> bool:
    return message.get("role") == "User" and not is_context_message(message)


def unchanged(messages: list[dict[str, Any]], reason: str) -> dict[str, Any]:
    return {
        "output": {
            "messages": messages,
            "changed": False,
            "summary": None,
            "token_estimate": None,
            "metadata": {
                "compactor": MODULE_ID,
                "input_messages": len(messages),
                "output_messages": len(messages),
                "skipped_reason": reason,
            },
        }
    }


def compact(params: Any) -> dict[str, Any]:
    compaction, trigger_messages, retain_user_turns = validate_input(params)
    messages = compaction["messages"]
    if len(messages) <= trigger_messages:
        return unchanged(messages, "below_trigger_messages")

    user_indices = [
        index for index, message in enumerate(messages) if is_user_message(message)
    ]
    if not user_indices:
        return unchanged(messages, "no_user_turn_boundary")

    retained_user_count = min(retain_user_turns, len(user_indices))
    suffix_start = user_indices[-retained_user_count]
    context_prefix = [
        message
        for index, message in enumerate(messages)
        if index < suffix_start and is_context_message(message)
    ]
    retained = context_prefix + messages[suffix_start:]
    if len(retained) >= len(messages):
        return unchanged(messages, "suffix_would_not_reduce_history")

    retained_users = sum(1 for message in retained if is_user_message(message))
    summary = (
        f"Deterministic suffix compaction retained {retained_users} recent user "
        f"turns and dropped {len(messages) - len(retained)} earlier messages."
    )
    return {
        "output": {
            "messages": retained,
            "changed": True,
            "summary": summary,
            "token_estimate": None,
            "metadata": {
                "compactor": MODULE_ID,
                "summary_source": "deterministic_suffix",
                "input_messages": len(messages),
                "output_messages": len(retained),
                "retained_user_turns": retained_users,
            },
        }
    }


def initialize(params: Any) -> dict[str, Any]:
    component_id = validate_initialize(params)
    return {
        "protocol_version": PROTOCOL_VERSION,
        "component_id": component_id,
        "exports": [
            {
                "slot": SLOT,
                "module_id": MODULE_ID,
                "contract_version": CONTRACT_VERSION,
                "composition": "select_one",
                "module_features": [],
            }
        ],
    }


def invoke(context: InvocationContext, method: str, params: Any) -> dict[str, Any]:
    if context.export != {"slot": SLOT, "module_id": MODULE_ID}:
        raise ProtocolError(f"unknown component export: {context.export!r}")
    if method != "compact":
        raise ProtocolError(f"unknown method: {method!r}")
    context.ensure_active()
    result = compact(params)
    context.ensure_active()
    return result


def main() -> int:
    run_component(initialize, invoke)


if __name__ == "__main__":
    raise SystemExit(main())
