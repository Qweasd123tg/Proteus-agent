#!/usr/bin/env python3
"""Dependency-free process HistoryCompactor example for Proteus.

The module keeps a valid suffix beginning at one of the most recent user
turns. Contract v1 permits the same host.model.complete callback for every
compactor, but this deterministic example does not need to call it.
"""

from __future__ import annotations

import json
import sys
from typing import Any


PROTOCOL_VERSION = "v2"
SLOT = "compactor"
MODULE_ID = "python_suffix"
CONTRACT_VERSION = "v1"

REQUEST_FIELDS = {"jsonrpc", "id", "method", "params"}
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
CALL_FIELDS = {"export", "params"}
EXPORT_REF_FIELDS = {"slot", "module_id"}
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


class ProtocolError(Exception):
    pass


def require_object(value: Any, fields: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ProtocolError(f"{label} must be an object")
    actual = set(value)
    if actual != fields:
        missing = sorted(fields - actual)
        unknown = sorted(actual - fields)
        raise ProtocolError(
            f"{label} fields mismatch: missing={missing}, unknown={unknown}"
        )
    return value


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


def unwrap_call(params: Any) -> Any:
    call = require_object(params, CALL_FIELDS, "component call")
    export = require_object(call["export"], EXPORT_REF_FIELDS, "component call export")
    if export != {"slot": SLOT, "module_id": MODULE_ID}:
        raise ProtocolError(f"unknown component export: {export!r}")
    return call["params"]


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


def response(request_id: Any, result: Any) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error_response(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def handle_request(raw: Any, component_id: str | None) -> tuple[dict[str, Any], str]:
    request = require_object(raw, REQUEST_FIELDS, "JSON-RPC request")
    if (
        request["jsonrpc"] != "2.0"
        or not isinstance(request["id"], (int, str))
        or isinstance(request["id"], bool)
    ):
        raise ProtocolError("unsupported JSON-RPC envelope")
    method = request["method"]
    if method == "initialize":
        if component_id is not None:
            raise ProtocolError("process component is already initialized")
        component_id = validate_initialize(request["params"])
        return response(
            request["id"],
            {
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
            },
        ), component_id
    if component_id is None:
        raise ProtocolError("initialize must be the first request")
    if method != "compact":
        raise ProtocolError(f"unknown method: {method!r}")
    return response(request["id"], compact(unwrap_call(request["params"]))), component_id


def main() -> int:
    component_id: str | None = None
    for line in sys.stdin:
        request_id: Any = None
        try:
            raw = json.loads(line)
            if isinstance(raw, dict):
                request_id = raw.get("id")
            result, component_id = handle_request(raw, component_id)
        except (ProtocolError, ValueError, TypeError) as error:
            result = error_response(request_id, -32602, str(error))
        except Exception as error:
            result = error_response(request_id, -32000, str(error))
        sys.stdout.write(json.dumps(result, separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
