#!/usr/bin/env python3
"""Dependency-free out-of-tree Workflow v2 component for Proteus.

The worker owns a small model/tool loop. Models, tools, policy, approvals,
safety, events, and cancellation remain host capabilities reached only through
the versioned bidirectional process-component protocol.
"""

from __future__ import annotations

import json
import sys
import uuid
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from component_runtime import (  # noqa: E402
    PROTOCOL_VERSION,
    HostError,
    InvocationCanceled,
    InvocationContext,
    ProtocolError,
    require_object,
    run_component,
)

SLOT = "workflow"
MODULE_ID = "python_agent_loop"
CONTRACT_VERSION = "v2"

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
INPUT_FIELDS = {"task", "history", "runtime"}
RUNTIME_FIELDS = {
    "session_id",
    "thread_id",
    "turn_id",
    "model_ref",
    "instructions",
    "reasoning",
    "max_input_tokens",
    "model_timeout_ms",
    "context_timeout_ms",
    "workflow_timeout_ms",
}
CONFIG_FIELDS = {"max_tool_rounds", "system_instructions"}


def require_string_list(value: Any, label: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ProtocolError(f"{label} must be an array of strings")
    return value


def parse_config(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ProtocolError("module config must be an object")
    config = value
    unknown = sorted(set(config) - CONFIG_FIELDS)
    if unknown:
        raise ProtocolError(f"unknown module config fields: {unknown}")
    max_rounds = config.get("max_tool_rounds", 8)
    if not isinstance(max_rounds, int) or isinstance(max_rounds, bool) or max_rounds <= 0:
        raise ProtocolError("module_config.max_tool_rounds must be a positive integer")
    instructions = config.get(
        "system_instructions",
        "You are a modular agent. Use tools when needed, then answer the user directly.",
    )
    if not isinstance(instructions, str):
        raise ProtocolError("module_config.system_instructions must be a string")
    return {"max_tool_rounds": max_rounds, "system_instructions": instructions}


class Peer:
    def __init__(self, context: InvocationContext) -> None:
        self.context = context

    def host_call(self, method: str, params: Any) -> Any:
        return self.context.host_call(method, params)

    def ensure_active(self) -> None:
        self.context.ensure_active()


def uuid_string() -> str:
    return str(uuid.uuid4())


def context_message(chunk: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": uuid_string(),
        "role": "User",
        "parts": [
            {
                "part_id": uuid_string(),
                "provenance": "context_builder",
                "scope": "request",
                "payload": {"Context": {"chunk": chunk}},
            }
        ],
        "name": "context",
        "tool_call_id": None,
        "metadata": None,
    }


def tool_result_message(result: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": uuid_string(),
        "role": "Tool",
        "parts": [
            {
                "part_id": uuid_string(),
                "provenance": "tool",
                "scope": "conversation",
                "payload": {"ToolResult": {"result": result}},
            }
        ],
        "name": None,
        "tool_call_id": result["call_id"],
        "metadata": None,
    }


def emit(peer: Peer, event: dict[str, Any]) -> None:
    acknowledgement = peer.host_call("host.events.emit", {"event": event})
    require_object(acknowledgement, set(), "event acknowledgement")


def runtime_status(peer: Peer) -> dict[str, Any]:
    status = require_object(
        peer.host_call("host.runtime.status", {}),
        {"cancelled", "queued_user_messages"},
        "runtime status",
    )
    if status["cancelled"]:
        raise InvocationCanceled("workflow invocation canceled by host")
    return status


def select_tools(peer: Peer, task: dict[str, Any]) -> list[dict[str, Any]]:
    output = peer.host_call(
        "host.tools.select",
        {
            "request": {
                "task": task,
                "cwd": task["cwd"],
                "query": None,
                "max_tools": None,
                "reason": "before_model_request",
                "phase": "process_agent_loop",
            }
        },
    )
    if not isinstance(output, dict) or not isinstance(output.get("tools"), list):
        raise ProtocolError("host.tools.select result must contain tools array")
    return output["tools"]


def model_request(
    invocation: dict[str, Any],
    messages: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    config: dict[str, Any],
) -> dict[str, Any]:
    runtime = invocation["runtime"]
    instructions = list(runtime["instructions"])
    if config["system_instructions"]:
        instructions.append(
            {
                "kind": "System",
                "text": config["system_instructions"],
                "priority": 100,
            }
        )
    return {
        "model": runtime["model_ref"],
        "instructions": instructions,
        "messages": messages,
        "tools": tools,
        "tool_choice": "Auto" if tools else "None",
        "response_format": "Text",
        "sampling": {"temperature": 0.0, "top_p": None},
        "reasoning": runtime["reasoning"],
        "limits": {
            "max_input_tokens": runtime["max_input_tokens"],
            "max_output_tokens": 16384,
        },
        "cache": {
            "cache_instructions": True,
            "cache_context": True,
            "routing_key": f"{runtime['session_id']}:{runtime['thread_id']}",
        },
        "client_metadata": {},
        "metadata": {"workflow": MODULE_ID},
    }


def complete_model(peer: Peer, request: dict[str, Any]) -> dict[str, Any]:
    emit(peer, {"ModelRequestPrepared": {"model": request["model"]}})
    response = peer.host_call("host.model.complete", {"request": request})
    response = require_object(
        response,
        {
            "messages",
            "tool_calls",
            "finish_reason",
            "usage",
            "end_turn",
            "provider_metadata",
        },
        "CanonicalModelResponse",
    )
    validate_model_response(response)
    emit(
        peer,
        {"ModelResponseReceived": {"finish_reason": response["finish_reason"]}},
    )
    return response


def validate_model_response(response: dict[str, Any]) -> None:
    messages = response["messages"]
    if not isinstance(messages, list) or not messages:
        raise ProtocolError("model response messages must be a non-empty array")
    if any(
        not isinstance(message, dict) or message.get("role") != "Assistant"
        for message in messages
    ):
        raise ProtocolError("model response messages must have Assistant role")
    calls = response["tool_calls"]
    if not isinstance(calls, list):
        raise ProtocolError("model response tool_calls must be an array")
    finish = response["finish_reason"]
    if finish == "ToolCalls" and not calls:
        raise ProtocolError("ToolCalls response must contain tool calls")
    if finish == "Stop" and calls:
        raise ProtocolError("Stop response must not contain tool calls")
    if finish not in {"ToolCalls", "Stop"}:
        raise ProtocolError(f"model response did not finish successfully: {finish}")
    message_calls = []
    for message in messages:
        for part in message.get("parts", []):
            payload = part.get("payload") if isinstance(part, dict) else None
            if isinstance(payload, dict) and set(payload) == {"ToolCall"}:
                body = payload["ToolCall"]
                if isinstance(body, dict):
                    message_calls.append(body.get("call"))
    if message_calls != calls:
        raise ProtocolError("assistant tool-call parts do not match response.tool_calls")


def execute_tools(
    peer: Peer, task: dict[str, Any], calls: list[dict[str, Any]]
) -> list[dict[str, Any]]:
    results = peer.host_call(
        "host.tools.execute_batch", {"task": task, "calls": calls}
    )
    if not isinstance(results, list) or len(results) != len(calls):
        raise ProtocolError("host.tools.execute_batch returned wrong result count")
    for call, result in zip(calls, results):
        if not isinstance(result, dict) or result.get("call_id") != call.get("id"):
            raise ProtocolError("tool result order/call_id does not match request")
    return results


def message_text(message: dict[str, Any]) -> str:
    texts: list[str] = []
    for part in message.get("parts", []):
        payload = part.get("payload") if isinstance(part, dict) else None
        if isinstance(payload, dict) and set(payload) == {"Text"}:
            body = payload["Text"]
            if isinstance(body, dict) and isinstance(body.get("text"), str):
                texts.append(body["text"])
    return "\n\n".join(texts)


def response_output_message(response: dict[str, Any]) -> dict[str, Any]:
    messages = response["messages"]
    for message in reversed(messages):
        if message_text(message).strip():
            return message
    return messages[-1]


def run_workflow(
    peer: Peer, params: Any, config: dict[str, Any]
) -> dict[str, Any]:
    invocation = require_object(params, INPUT_FIELDS, "ProcessWorkflowInput")
    runtime = require_object(invocation["runtime"], RUNTIME_FIELDS, "workflow runtime")
    task = require_object(invocation["task"], {"text", "cwd"}, "AgentTask")
    history = invocation["history"]
    if not isinstance(history, list) or not history:
        raise ProtocolError("workflow history must be a non-empty array")
    if history[-1].get("role") != "User":
        raise ProtocolError("workflow history must end with the current user message")
    del runtime

    runtime_status(peer)
    emit(peer, {"TaskReceived": {"task": task}})
    context = peer.host_call("host.context.build", {"task": task})
    if not isinstance(context, dict) or not isinstance(context.get("chunks"), list):
        raise ProtocolError("host.context.build result must contain chunks array")
    emit(
        peer,
        {
            "ContextBuilt": {
                "chunks": len(context["chunks"]),
                "token_estimate": context.get("token_estimate"),
            }
        },
    )

    messages = [context_message(chunk) for chunk in context["chunks"]]
    messages.extend(history)
    persistent_new: list[dict[str, Any]] = []
    tools = select_tools(peer, task)
    tool_rounds = 0

    while True:
        runtime_status(peer)
        final_round = tool_rounds >= config["max_tool_rounds"]
        request = model_request(
            invocation,
            messages,
            [] if final_round else tools,
            config,
        )
        response = complete_model(peer, request)
        assistant_messages = response["messages"]
        messages.extend(assistant_messages)
        persistent_new.extend(assistant_messages)

        if response["finish_reason"] == "Stop":
            assistant = response_output_message(response)
            text = message_text(assistant)
            output = {
                "text": text,
                "metadata": {
                    "workflow": MODULE_ID,
                    "tool_rounds": tool_rounds,
                    "context_chunks": len(context["chunks"]),
                },
            }
            emit(peer, {"TurnFinished": {"output": output}})
            return {
                "result": {
                    "output": output,
                    "new_messages": persistent_new,
                    "history_replacement": None,
                    "compactions": [],
                }
            }

        if final_round:
            raise ProtocolError("model returned tool calls after tools were disabled")
        results = execute_tools(peer, task, response["tool_calls"])
        for result in results:
            message = tool_result_message(result)
            messages.append(message)
            persistent_new.append(message)
        tool_rounds += 1


component_config: dict[str, Any] = {}


def initialize(raw: Any) -> dict[str, Any]:
    global component_config
    params = require_object(raw, INITIALIZE_FIELDS, "initialize params")
    if params["protocol_version"] != PROTOCOL_VERSION:
        raise ProtocolError(f"unsupported component protocol: {params['protocol_version']!r}")
    component_id = params["component_id"]
    if not isinstance(component_id, str) or not component_id.strip():
        raise ProtocolError("initialize component_id must be a non-empty string")
    exports = params["exports"]
    if not isinstance(exports, list) or len(exports) != 1:
        raise ProtocolError("python agent component requires exactly one export")
    export = require_object(
        exports[0], EXPORT_INITIALIZE_FIELDS, "initialize export"
    )
    expected = (SLOT, MODULE_ID, CONTRACT_VERSION, "select_one")
    actual = (
        export["slot"],
        export["module_id"],
        export["contract_version"],
        export["composition"],
    )
    if actual != expected:
        raise ProtocolError(f"unsupported initialize identity: {actual!r}")
    if require_string_list(export["host_features"], "host_features"):
        raise ProtocolError("workflow v2 has no negotiated optional features")
    component_config = parse_config(export["module_config"])
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
    if method != "run":
        raise ProtocolError(f"workflow v2 does not support method {method!r}")
    return run_workflow(Peer(context), params, component_config)


def main() -> int:
    run_component(initialize, invoke)


if __name__ == "__main__":
    raise SystemExit(main())
