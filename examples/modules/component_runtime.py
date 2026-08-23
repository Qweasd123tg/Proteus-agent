"""Dependency-free Component Runtime v3 helper for Python examples.

The helper is not a Proteus SDK dependency: it is small example code for the
newline-JSON wire contract. One thread owns stdin, invocation work is bounded
and concurrent, stdout is serialized, cancellation is invocation-scoped, and
callback responses are routed to per-call queues.
"""

from __future__ import annotations

import json
import queue
import sys
import threading
from collections.abc import Callable
from typing import Any, NoReturn


PROTOCOL_VERSION = "v3"
CANCEL_METHOD = "$/cancelRequest"
CANCELLED_CODE = -32800
MAX_ACTIVE_INVOCATIONS = 32
MAX_PENDING_CALLBACKS = 256


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


def parse_wire_id(value: Any, direction: str, generation: int, allow_zero: bool) -> int:
    if not isinstance(value, str):
        raise ProtocolError("component wire id must be a string")
    parts = value.split(":")
    if len(parts) != 3 or parts[0] != direction:
        raise ProtocolError(f"component wire id has wrong direction: {value!r}")
    for part in parts[1:]:
        if not part.isdigit() or str(int(part)) != part:
            raise ProtocolError(f"component wire id is not canonical: {value!r}")
    if int(parts[1]) != generation:
        raise ProtocolError(f"component wire id has stale generation: {value!r}")
    sequence = int(parts[2])
    if sequence == 0 and not allow_zero:
        raise ProtocolError(f"component wire id sequence must be non-zero: {value!r}")
    return sequence


def compact_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def result_response(request_id: str, result: Any) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def error_response(request_id: str, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


class InvocationContext:
    def __init__(
        self,
        runtime: "ComponentRuntime",
        invocation_id: str,
        export: dict[str, str],
        lineage: dict[str, Any],
    ) -> None:
        self._runtime = runtime
        self.invocation_id = invocation_id
        self.export = export
        self.lineage = lineage
        self._cancelled = threading.Event()
        self._cancel_callbacks: list[Callable[[], None]] = []
        self._cancel_lock = threading.Lock()

    def is_cancelled(self) -> bool:
        return self._cancelled.is_set()

    def ensure_active(self) -> None:
        if self.is_cancelled():
            raise InvocationCanceled(
                f"invocation {self.invocation_id} was canceled by host"
            )

    def on_cancel(self, callback: Callable[[], None]) -> None:
        run_now = False
        with self._cancel_lock:
            if self.is_cancelled():
                run_now = True
            else:
                self._cancel_callbacks.append(callback)
        if run_now:
            callback()

    def cancel(self) -> None:
        if self._cancelled.is_set():
            return
        self._cancelled.set()
        with self._cancel_lock:
            callbacks = list(self._cancel_callbacks)
            self._cancel_callbacks.clear()
        for callback in callbacks:
            try:
                callback()
            except Exception:
                pass

    def host_call(self, method: str, params: Any) -> Any:
        self.ensure_active()
        result = self._runtime.host_call(self.invocation_id, method, params)
        self.ensure_active()
        return result


Initialize = Callable[[Any], dict[str, Any]]
Invoke = Callable[[InvocationContext, str, Any], Any]


class ComponentRuntime:
    def __init__(self, initialize: Initialize, invoke: Invoke) -> None:
        self._initialize = initialize
        self._invoke = invoke
        self._generation = 0
        self._writer_lock = threading.Lock()
        self._active_lock = threading.Lock()
        self._active: dict[str, InvocationContext] = {}
        self._callback_lock = threading.Lock()
        self._callbacks: dict[str, queue.Queue[tuple[bool, Any]]] = {}
        self._next_callback = 1

    def run(self) -> int:
        first = sys.stdin.readline()
        if not first:
            return 1
        request_id = "h:0:0"
        try:
            request = require_object(
                json.loads(first),
                {"jsonrpc", "id", "method", "params"},
                "initialize request",
            )
            request_id = request["id"]
            self._validate_jsonrpc(request)
            if request["method"] != "initialize":
                raise ProtocolError("first request must be initialize")
            self._generation = self._initialize_generation(request_id)
            manifest = self._initialize(request["params"])
            self.send(result_response(request_id, manifest))
        except Exception as error:
            self.send(error_response(request_id, -32602, str(error)))
            return 2

        for line in sys.stdin:
            if not line.strip():
                continue
            self._route(json.loads(line))
        return 0

    def send(self, value: Any) -> None:
        with self._writer_lock:
            print(compact_json(value), flush=True)

    def host_call(self, invocation_id: str, method: str, params: Any) -> Any:
        with self._callback_lock:
            if len(self._callbacks) >= MAX_PENDING_CALLBACKS:
                raise HostError("pending callback capacity is exhausted")
            sequence = self._next_callback
            self._next_callback += 1
            callback_id = f"m:{self._generation}:{sequence}"
            result_queue: queue.Queue[tuple[bool, Any]] = queue.Queue(maxsize=1)
            self._callbacks[callback_id] = result_queue
        self.send(
            {
                "jsonrpc": "2.0",
                "id": callback_id,
                "method": method,
                "params": {"invocation_id": invocation_id, "params": params},
            }
        )
        succeeded, value = result_queue.get()
        if succeeded:
            return value
        if not isinstance(value, dict):
            raise HostError(f"{method}: callback error must be an object")
        error = (
            require_object(value, {"code", "message"}, "callback error")
            if set(value) == {"code", "message"}
            else require_object(
                value, {"code", "message", "data"}, "callback error"
            )
        )
        raise HostError(
            f"{method}: JSON-RPC {error['code']}: {error['message']}"
        )

    def _route(self, raw: Any) -> None:
        if not isinstance(raw, dict):
            raise ProtocolError("JSON-RPC frame must be an object")
        shape = ("id" in raw, "method" in raw, "result" in raw, "error" in raw)
        if shape == (True, True, False, False):
            request = require_object(
                raw, {"jsonrpc", "id", "method", "params"}, "invocation request"
            )
            self._validate_jsonrpc(request)
            self._start_invocation(request)
            return
        if shape == (False, True, False, False):
            notification = require_object(
                raw, {"jsonrpc", "method", "params"}, "component notification"
            )
            self._validate_jsonrpc(notification)
            self._cancel(notification)
            return
        if shape in ((True, False, True, False), (True, False, False, True)):
            fields = {"jsonrpc", "id", "result" if "result" in raw else "error"}
            response = require_object(raw, fields, "callback response")
            self._validate_jsonrpc(response)
            self._complete_callback(response)
            return
        raise ProtocolError("invalid or ambiguous JSON-RPC envelope")

    def _start_invocation(self, request: dict[str, Any]) -> None:
        invocation_id = request["id"]
        parse_wire_id(invocation_id, "h", self._generation, False)
        if not isinstance(request["method"], str) or not request["method"].strip():
            raise ProtocolError("invocation method must be a non-empty string")
        invocation = require_object(
            request["params"], {"export", "lineage", "params"}, "invocation params"
        )
        export = require_object(
            invocation["export"], {"slot", "module_id"}, "invocation export"
        )
        if not all(isinstance(export[field], str) for field in export):
            raise ProtocolError("invocation export fields must be strings")
        lineage = require_object(
            invocation["lineage"],
            {"root_invocation_id", "parent_invocation_id", "depth"},
            "invocation lineage",
        )
        context = InvocationContext(self, invocation_id, export, lineage)
        with self._active_lock:
            if invocation_id in self._active:
                raise ProtocolError(f"invocation id was reused: {invocation_id}")
            self._validate_lineage_locked(invocation_id, lineage)
            if len(self._active) >= MAX_ACTIVE_INVOCATIONS:
                self.send(
                    error_response(
                        invocation_id,
                        -32014,
                        "active invocation capacity is exhausted",
                    )
                )
                return
            self._active[invocation_id] = context
        thread = threading.Thread(
            target=self._execute,
            args=(context, request["method"], invocation["params"]),
            name=f"component-invocation-{invocation_id.replace(':', '-')}",
            daemon=True,
        )
        try:
            thread.start()
        except RuntimeError as error:
            with self._active_lock:
                self._active.pop(invocation_id, None)
            self.send(
                error_response(
                    invocation_id,
                    -32603,
                    f"failed to start invocation thread: {error}",
                )
            )

    def _execute(self, context: InvocationContext, method: str, params: Any) -> None:
        try:
            result = self._invoke(context, method, params)
            context.ensure_active()
            response = result_response(context.invocation_id, result)
        except InvocationCanceled as error:
            response = error_response(context.invocation_id, CANCELLED_CODE, str(error))
        except (ProtocolError, HostError) as error:
            code = CANCELLED_CODE if context.is_cancelled() else -32602
            response = error_response(context.invocation_id, code, str(error))
        except Exception as error:
            code = CANCELLED_CODE if context.is_cancelled() else -32603
            response = error_response(context.invocation_id, code, str(error))
        self.send(response)
        with self._active_lock:
            self._active.pop(context.invocation_id, None)

    def _cancel(self, notification: dict[str, Any]) -> None:
        if notification["method"] != CANCEL_METHOD:
            raise ProtocolError(
                f"unsupported component notification: {notification['method']!r}"
            )
        params = require_object(
            notification["params"], {"invocation_id", "cause"}, "cancel params"
        )
        invocation_id = params["invocation_id"]
        parse_wire_id(invocation_id, "h", self._generation, False)
        if params["cause"] not in {"user", "timeout", "shutdown"}:
            raise ProtocolError("cancel cause is invalid")
        with self._active_lock:
            context = self._active.get(invocation_id)
        if context is not None:
            context.cancel()

    def _complete_callback(self, response: dict[str, Any]) -> None:
        callback_id = response["id"]
        parse_wire_id(callback_id, "m", self._generation, False)
        with self._callback_lock:
            result_queue = self._callbacks.pop(callback_id, None)
        if result_queue is None:
            raise ProtocolError(
                f"callback response references unknown id: {callback_id!r}"
            )
        if "result" in response:
            result_queue.put((True, response["result"]))
        else:
            result_queue.put((False, response["error"]))

    def _validate_lineage_locked(
        self, invocation_id: str, lineage: dict[str, Any]
    ) -> None:
        root_id = lineage["root_invocation_id"]
        parent_id = lineage["parent_invocation_id"]
        depth = lineage["depth"]
        parse_wire_id(root_id, "h", self._generation, False)
        if not isinstance(depth, int) or isinstance(depth, bool) or depth < 0:
            raise ProtocolError("lineage depth must be a non-negative integer")
        if depth == 0:
            if root_id != invocation_id or parent_id is not None:
                raise ProtocolError("root invocation has inconsistent lineage")
            return
        parse_wire_id(parent_id, "h", self._generation, False)
        parent = self._active.get(parent_id)
        if parent is None:
            raise ProtocolError("nested invocation names an inactive parent")
        if (
            root_id != parent.lineage["root_invocation_id"]
            or depth != parent.lineage["depth"] + 1
        ):
            raise ProtocolError("nested invocation has inconsistent lineage")

    def _initialize_generation(self, request_id: Any) -> int:
        if not isinstance(request_id, str):
            raise ProtocolError("initialize id must be a string")
        parts = request_id.split(":")
        if len(parts) != 3 or parts[0] != "h":
            raise ProtocolError("initialize id must be host-directed")
        if not all(part.isdigit() and str(int(part)) == part for part in parts[1:]):
            raise ProtocolError("initialize id must be canonical")
        if int(parts[2]) != 0:
            raise ProtocolError("initialize id sequence must be zero")
        return int(parts[1])

    @staticmethod
    def _validate_jsonrpc(message: dict[str, Any]) -> None:
        if message.get("jsonrpc") != "2.0":
            raise ProtocolError("JSON-RPC version must be 2.0")


def run_component(initialize: Initialize, invoke: Invoke) -> NoReturn:
    raise SystemExit(ComponentRuntime(initialize, invoke).run())
