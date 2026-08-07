#!/usr/bin/env python3
"""Dependency-free out-of-tree Workflow v1 agent worker for Proteus.

The worker owns a small model/tool loop. Models, tools, policy, approvals,
safety, events, and cancellation remain host capabilities reached only through
the versioned bidirectional process-module protocol.
"""

from __future__ import annotations

import json
import sys
import uuid
from typing import Any, NoReturn


PROTOCOL_VERSION = "v1"
SLOT = "workflow"
MODULE_ID = "python_agent_loop"
CONTRACT_VERSION = "v1"
CANCEL_METHOD = "$/cancelRequest"
CANCELLED_CODE = -32800

INITIALIZE_FIELDS = {
    "protocol_version",
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


class ProtocolError(Exception):
    pass


class HostError(Exception):
    pass


class InvocationCanceled(Exception):
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


def require_string_list(value: Any, label: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ProtocolError(f"{label} must be an array of strings")
    return value


def compact_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def send(value: Any) -> None:
    print(compact_json(value), flush=True)


def result_response(request_id: Any, result: Any) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error_response(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


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
    def __init__(self) -> None:
        self.next_host_id = 1
        self.active_invocation: str | None = None
        self.cancelled = False

    def host_call(self, method: str, params: Any) -> Any:
        self.ensure_active()
        request_id = f"host-{self.next_host_id}"
        self.next_host_id += 1
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": method,
                "params": params,
            }
        )
        while True:
            line = sys.stdin.readline()
            if not line:
                raise EOFError("host closed stdin while callback was pending")
            if not line.strip():
                continue
            message = json.loads(line)
            if isinstance(message, dict) and message.get("method") == CANCEL_METHOD:
                self.accept_cancel(message)
                self.ensure_active()
                continue
            if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
                raise ProtocolError("host callback response must be JSON-RPC 2.0")
            if message.get("id") != request_id:
                raise ProtocolError(
                    f"callback response id {message.get('id')!r} did not match {request_id!r}"
                )
            if set(message) == {"jsonrpc", "id", "result"}:
                return message["result"]
            if set(message) == {"jsonrpc", "id", "error"}:
                error = message["error"]
                if not isinstance(error, dict):
                    raise ProtocolError("host callback error must be an object")
                raise HostError(
                    f"{method}: JSON-RPC {error.get('code')}: {error.get('message')}"
                )
            raise ProtocolError("host callback response fields mismatch")

    def accept_cancel(self, message: dict[str, Any]) -> None:
        if set(message) != {"jsonrpc", "method", "params"}:
            raise ProtocolError("cancel notification fields mismatch")
        params = require_object(message["params"], {"id"}, "cancel params")
        if params["id"] == self.active_invocation:
            self.cancelled = True

    def ensure_active(self) -> None:
        if self.cancelled:
            raise InvocationCanceled("workflow invocation canceled by host")


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
            "message",
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
    message = response["message"]
    if not isinstance(message, dict) or message.get("role") != "Assistant":
        raise ProtocolError("model response message must have Assistant role")
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
        assistant = response["message"]
        messages.append(assistant)
        persistent_new.append(assistant)

        if response["finish_reason"] == "Stop":
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


def initialize(raw: Any) -> dict[str, Any]:
    request = require_object(raw, {"jsonrpc", "id", "method", "params"}, "initialize request")
    if request["jsonrpc"] != "2.0" or request["method"] != "initialize":
        raise ProtocolError("first request must be JSON-RPC initialize")
    params = require_object(request["params"], INITIALIZE_FIELDS, "initialize params")
    expected = (PROTOCOL_VERSION, SLOT, MODULE_ID, CONTRACT_VERSION, "select_one")
    actual = (
        params["protocol_version"],
        params["slot"],
        params["module_id"],
        params["contract_version"],
        params["composition"],
    )
    if actual != expected:
        raise ProtocolError(f"unsupported initialize identity: {actual!r}")
    if require_string_list(params["host_features"], "host_features"):
        raise ProtocolError("workflow v1 has no negotiated optional features")
    config = parse_config(params["module_config"])
    manifest = {
        "protocol_version": PROTOCOL_VERSION,
        "slot": SLOT,
        "module_id": MODULE_ID,
        "contract_version": CONTRACT_VERSION,
        "composition": "select_one",
        "module_features": [],
    }
    send(result_response(request["id"], manifest))
    return config


def fail_initialization(request_id: Any, error: Exception) -> NoReturn:
    send(error_response(request_id, -32602, str(error)))
    raise SystemExit(2)


def main() -> int:
    first = sys.stdin.readline()
    if not first:
        return 1
    request_id: Any = None
    try:
        raw = json.loads(first)
        if isinstance(raw, dict):
            request_id = raw.get("id")
        config = initialize(raw)
    except Exception as error:
        fail_initialization(request_id, error)

    peer = Peer()
    for line in sys.stdin:
        if not line.strip():
            continue
        request_id: Any = None
        try:
            raw = json.loads(line)
            if isinstance(raw, dict) and raw.get("method") == CANCEL_METHOD:
                peer.accept_cancel(raw)
                continue
            request = require_object(
                raw, {"jsonrpc", "id", "method", "params"}, "workflow request"
            )
            request_id = request["id"]
            if request["jsonrpc"] != "2.0" or request["method"] != "run":
                raise ProtocolError("workflow v1 supports only method run")
            if not isinstance(request_id, str):
                raise ProtocolError("workflow invocation id must be a string")
            peer.active_invocation = request_id
            peer.cancelled = False
            output = run_workflow(peer, request["params"], config)
            send(result_response(request_id, output))
        except InvocationCanceled as error:
            send(error_response(request_id, CANCELLED_CODE, str(error)))
        except (ProtocolError, HostError) as error:
            send(error_response(request_id, -32602, str(error)))
        except Exception as error:
            send(error_response(request_id, -32603, str(error)))
        finally:
            peer.active_invocation = None
            peer.cancelled = False
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
