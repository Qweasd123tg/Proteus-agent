#!/usr/bin/env python3
"""Dependency-free process HistoryCompactor example for Proteus.

The module keeps a valid suffix beginning at one of the most recent user
turns. It deliberately performs no model call: process compactors are pure
transforms and receive no CompactionHost capabilities.
"""

from __future__ import annotations

import json
import sys
from typing import Any


PROTOCOL_VERSION = "v0"
SLOT = "compactor"
MODULE_ID = "python_suffix"
CONTRACT_VERSION = "v0"

REQUEST_FIELDS = {"jsonrpc", "id", "method", "params"}
INITIALIZE_FIELDS = {"protocol_version", "slot", "contract_version"}
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


def validate_initialize(params: Any) -> None:
    params = require_object(params, INITIALIZE_FIELDS, "initialize params")
    expected = {
        "protocol_version": PROTOCOL_VERSION,
        "slot": SLOT,
        "contract_version": CONTRACT_VERSION,
    }
    if params != expected:
        raise ProtocolError(f"unsupported initialize params: {params!r}")


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


def response(request_id: int, result: Any) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error_response(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def handle_request(raw: Any, initialized: bool) -> tuple[dict[str, Any], bool]:
    request = require_object(raw, REQUEST_FIELDS, "JSON-RPC request")
    if request["jsonrpc"] != "2.0" or not isinstance(request["id"], int):
        raise ProtocolError("unsupported JSON-RPC envelope")
    method = request["method"]
    if method == "initialize":
        if initialized:
            raise ProtocolError("process module is already initialized")
        validate_initialize(request["params"])
        return response(
            request["id"],
            {
                "protocol_version": PROTOCOL_VERSION,
                "slot": SLOT,
                "module_id": MODULE_ID,
                "contract_version": CONTRACT_VERSION,
            },
        ), True
    if not initialized:
        raise ProtocolError("initialize must be the first request")
    if method != "compact":
        raise ProtocolError(f"unknown method: {method!r}")
    return response(request["id"], compact(request["params"])), initialized


def main() -> int:
    initialized = False
    for line in sys.stdin:
        request_id: Any = None
        try:
            raw = json.loads(line)
            if isinstance(raw, dict):
                request_id = raw.get("id")
            result, initialized = handle_request(raw, initialized)
        except (ProtocolError, ValueError, TypeError) as error:
            result = error_response(request_id, -32602, str(error))
        except Exception as error:
            result = error_response(request_id, -32000, str(error))
        sys.stdout.write(json.dumps(result, separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
