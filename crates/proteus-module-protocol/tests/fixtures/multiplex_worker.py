#!/usr/bin/env python3
"""Small non-Rust fixture for the Component Runtime v2 broker tests.

This is deliberately a test fixture, not a worker SDK.  It implements the
minimum useful worker shape in plain Python:

* one stdin reader that keeps accepting frames while calls are running;
* one daemon thread per host invocation;
* a lock around complete JSON-lines writes; and
* per-invocation cancellation and callback wait state.

The P0 harness uses ``params.input``; the production-v3 suite uses the strict
``params.params`` wrapper. The fixture keeps those test contracts explicit and
does not act as a compatibility layer for production code.

    {"jsonrpc":"2.0", "id":"h:1:1", "method":"run",
     "params":{"export": {...}, "lineage": {...}, "input":
        {"op":"echo", "value":"hello"}}}

The worker intentionally accepts any host request method with an ``h:*`` id:
P0 is exercising transport semantics rather than a particular slot method.
"""

from __future__ import annotations

import json
import os
import sys
import threading
import time
from dataclasses import dataclass, field
from typing import Any, Dict, Optional


JSON = Dict[str, Any]


@dataclass
class CallbackWaiter:
    event: threading.Event = field(default_factory=threading.Event)
    response: Optional[JSON] = None


@dataclass
class Invocation:
    invocation_id: str
    request: JSON
    cancel: threading.Event = field(default_factory=threading.Event)


class Worker:
    def __init__(self) -> None:
        self._stdout_lock = threading.Lock()
        self._state_lock = threading.Lock()
        self._invocations: Dict[str, Invocation] = {}
        self._callbacks: Dict[str, CallbackWaiter] = {}
        self._next_callback = 0
        self._protocol_version = "component-v3-spike"

    def send(self, frame: JSON) -> None:
        """Write exactly one JSON frame; invocation threads never interleave bytes."""
        encoded = json.dumps(frame, separators=(",", ":"), sort_keys=True)
        with self._stdout_lock:
            sys.stdout.write(encoded + "\n")
            sys.stdout.flush()

    def next_callback_id(self, invocation_id: str) -> str:
        generation = invocation_id.split(":", 2)[1] if invocation_id.count(":") >= 2 else "0"
        with self._state_lock:
            self._next_callback += 1
            return f"m:{generation}:{self._next_callback}"

    def receive(self, frame: JSON) -> None:
        if "id" in frame and "method" not in frame:
            self._resolve_callback(frame)
            return

        method = frame.get("method")
        if method == "$/cancelRequest":
            self._cancel(frame.get("params", {}))
            return

        if method == "initialize":
            self._initialize(frame)
            return

        invocation_id = frame.get("id")
        if isinstance(method, str) and isinstance(invocation_id, str) and invocation_id.startswith("h:"):
            invocation = Invocation(invocation_id=invocation_id, request=frame)
            with self._state_lock:
                # The broker must reject duplicate active ids.  This fixture
                # emits a duplicate response only when a test explicitly asks
                # for it; otherwise retaining the original avoids hiding it.
                if invocation_id in self._invocations:
                    self.send_error(invocation_id, -32600, "duplicate active host invocation id")
                    return
                self._invocations[invocation_id] = invocation
            threading.Thread(
                target=self._run_invocation,
                args=(invocation,),
                name=f"fixture-{invocation_id}",
                daemon=True,
            ).start()
            if self._input(frame).get("op") == "stop_reading":
                # Hostile P0 case: stdout/invocation threads remain alive while
                # the sole stdin owner stops consuming frames.
                time.sleep(self._milliseconds(self._input(frame)) or 60.0)

    def _initialize(self, frame: JSON) -> None:
        """Answer either the P0 spike handshake or the exact P2 v3 manifest."""
        request_id = frame.get("id")
        params = frame.get("params")
        if not isinstance(request_id, str) or not request_id.startswith("h:"):
            return
        if not isinstance(params, dict):
            self.send_error(request_id, -32602, "initialize params must be an object")
            return
        protocol_version = params.get("protocol_version")
        if protocol_version == "v3":
            component_id = params.get("component_id")
            exports = params.get("exports")
            if not isinstance(component_id, str) or not isinstance(exports, list):
                self.send_error(request_id, -32602, "invalid component-v3 binding")
                return
            self._protocol_version = "v3"
            manifest_exports = []
            for export in exports:
                if not isinstance(export, dict):
                    self.send_error(request_id, -32602, "invalid component-v3 export")
                    return
                manifest_exports.append({
                    "slot": export.get("slot"),
                    "module_id": export.get("module_id"),
                    "contract_version": export.get("contract_version"),
                    "composition": export.get("composition"),
                    "module_features": [],
                })
            self.send({
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocol_version": "v3",
                    "component_id": component_id,
                    "exports": manifest_exports,
                },
            })
            return
        if protocol_version != "component-v3-spike":
            self.send_error(request_id, -32602, "unknown component protocol")
            return
        self._protocol_version = "component-v3-spike"
        self.send({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {
                "protocol_version": "component-v3-spike",
                "pid": os.getpid(),
                "capabilities": [
                    "concurrent_invocations",
                    "targeted_cancel",
                    "host_callbacks",
                    "progress_notifications",
                ],
            },
        })

    def _resolve_callback(self, response: JSON) -> None:
        callback_id = response.get("id")
        if not isinstance(callback_id, str):
            return
        with self._state_lock:
            waiter = self._callbacks.get(callback_id)
        if waiter is not None:
            waiter.response = response
            waiter.event.set()

    def _cancel(self, params: Any) -> None:
        if not isinstance(params, dict):
            return
        invocation_id = params.get("invocation_id", params.get("id"))
        if not isinstance(invocation_id, str):
            return
        with self._state_lock:
            invocation = self._invocations.get(invocation_id)
        if invocation is not None:
            invocation.cancel.set()

    @staticmethod
    def _input(request: JSON) -> JSON:
        params = request.get("params")
        if not isinstance(params, dict):
            return {}
        input_value = params.get("params", params.get("input"))
        return input_value if isinstance(input_value, dict) else {}

    def _is_v3(self) -> bool:
        return self._protocol_version == "v3"

    def _callback_method(self, invocation: Invocation) -> str:
        if not self._is_v3():
            return "host.nested.invoke"
        params = invocation.request.get("params")
        export = params.get("export") if isinstance(params, dict) else None
        slot = export.get("slot") if isinstance(export, dict) else None
        return "host.search.query" if slot == "context" else "host.context.build"

    def _callback_params(
        self,
        invocation: Invocation,
        input_value: JSON,
        parent_id: Optional[Any] = None,
    ) -> JSON:
        payload = input_value.get("callback_input", {"op": "echo"})
        params: JSON = {
            "invocation_id": parent_id if parent_id is not None else invocation.invocation_id,
        }
        params["params" if self._is_v3() else "input"] = payload
        return params

    @staticmethod
    def _milliseconds(payload: JSON, key: str = "delay_ms") -> float:
        value = payload.get(key, 0)
        return max(0.0, float(value)) / 1000.0 if isinstance(value, (int, float)) else 0.0

    def _sleep_or_cancel(self, invocation: Invocation, seconds: float) -> bool:
        return invocation.cancel.wait(seconds)

    def _run_invocation(self, invocation: Invocation) -> None:
        input_value = self._input(invocation.request)
        operation = input_value.get("op", "echo")
        if not isinstance(operation, str):
            operation = "echo"
        try:
            admission_delay = self._milliseconds(input_value, "admission_delay_ms")
            if admission_delay and self._sleep_or_cancel(invocation, admission_delay):
                self.send_canceled(invocation.invocation_id)
                return

            if operation == "forged_parent":
                response = self._callback(
                    invocation,
                    input_value,
                    parent_id=input_value.get("forged_parent_id", "h:forged:1"),
                )
                self._finish_callback(invocation, response)
                return
            if operation == "sibling_parent":
                response = self._callback(
                    invocation,
                    input_value,
                    parent_id=input_value.get("other_active_id", "h:other:1"),
                )
                self._finish_callback(invocation, response)
                return
            if operation == "queued_parent_callback":
                time.sleep(self._milliseconds(input_value) or 0.1)
                response = self._callback(
                    invocation,
                    input_value,
                    parent_id=self._offset_invocation_id(invocation.invocation_id, 1),
                )
                self._finish_callback(invocation, response)
                return
            if operation == "queued_parent_terminal":
                time.sleep(self._milliseconds(input_value) or 0.1)
                self.send_result(
                    self._offset_invocation_id(invocation.invocation_id, 1),
                    {"forged_queued_terminal": True},
                )
                return
            if operation == "queued_parent_notification":
                time.sleep(self._milliseconds(input_value) or 0.1)
                self.progress(
                    self._offset_invocation_id(invocation.invocation_id, 1),
                    0,
                    {"forged_queued_notification": True},
                )
                return
            if operation == "duplicate_callback_id":
                self._duplicate_callback(invocation, input_value)
                return
            if operation == "terminal_during_callback":
                self._terminal_during_callback(invocation, input_value)
                return
            if operation == "wrong_callback_direction":
                self._wrong_callback_direction(invocation, input_value)
                return
            if operation == "terminal_then_late":
                self.send_result(invocation.invocation_id, {"terminal": True})
                time.sleep(self._milliseconds(input_value, "late_delay_ms") or 0.01)
                self.progress(invocation.invocation_id, 0, {"late": True})
                return
            if operation == "duplicate_terminal":
                self.send_result(invocation.invocation_id, {"terminal": "first"})
                self.send_result(invocation.invocation_id, {"terminal": "second"})
                return
            if operation == "malformed":
                with self._stdout_lock:
                    sys.stdout.write("{this-is-not-json\n")
                    sys.stdout.flush()
                return
            if operation == "oversized":
                size = input_value.get("bytes", 2 * 1024 * 1024)
                size = size if isinstance(size, int) else 2 * 1024 * 1024
                self.progress(invocation.invocation_id, 0, "x" * max(0, size))
                return
            if operation == "flood":
                count = input_value.get("count", 128)
                count = count if isinstance(count, int) else 128
                for sequence in range(max(0, count)):
                    self.progress(invocation.invocation_id, sequence, input_value.get("payload", "x"))
                if invocation.cancel.is_set():
                    self.send_canceled(invocation.invocation_id)
                else:
                    self.send_result(invocation.invocation_id, {"flooded": max(0, count)})
                return
            if operation == "callback":
                response = self._callback(invocation, input_value)
                self._finish_callback(invocation, response)
                return
            if operation == "lineage":
                params = invocation.request.get("params")
                lineage = params.get("lineage") if isinstance(params, dict) else None
                self.send_result(invocation.invocation_id, lineage)
                return

            delay = self._milliseconds(input_value)
            if operation == "exit_process":
                if delay:
                    time.sleep(delay)
                # P0 needs a worker that dies without a terminal response.
                # os._exit deliberately bypasses thread cleanup and stdio
                # flushing, matching an abrupt component process loss.
                os._exit(23)
            if operation == "stop_reading":
                self.progress(invocation.invocation_id, 0, {"stdin_stopped": True})
                time.sleep(delay if delay else 60.0)
                self.send_result(invocation.invocation_id, {"stdin_resumed": True})
                return
            if operation == "wait_cancel":
                invocation.cancel.wait(delay if delay else 60.0)
                if invocation.cancel.is_set():
                    self.send_canceled(invocation.invocation_id)
                else:
                    self.send_result(invocation.invocation_id, {"waited": True})
                return
            if operation == "ignore_cancel":
                # Deliberately uncooperative: only a broker reset can stop it.
                time.sleep(delay if delay else 60.0)
                self.send_result(invocation.invocation_id, {"ignored_cancel": True})
                return
            if delay and self._sleep_or_cancel(invocation, delay):
                self.send_canceled(invocation.invocation_id)
                return
            if invocation.cancel.is_set():
                self.send_canceled(invocation.invocation_id)
                return
            self.send_result(invocation.invocation_id, input_value.get("value", input_value))
        finally:
            with self._state_lock:
                self._invocations.pop(invocation.invocation_id, None)

    def _callback(
        self,
        invocation: Invocation,
        input_value: JSON,
        parent_id: Optional[Any] = None,
    ) -> Optional[JSON]:
        callback_id = self.next_callback_id(invocation.invocation_id)
        waiter = CallbackWaiter()
        with self._state_lock:
            self._callbacks[callback_id] = waiter
        try:
            self.send({
                "jsonrpc": "2.0",
                "id": callback_id,
                "method": self._callback_method(invocation),
                "params": self._callback_params(invocation, input_value, parent_id),
            })
            timeout = self._milliseconds(input_value, "callback_timeout_ms") or 5.0
            deadline = time.monotonic() + timeout
            while not waiter.event.wait(min(0.02, max(0.0, deadline - time.monotonic()))):
                if invocation.cancel.is_set() or time.monotonic() >= deadline:
                    return None
            return waiter.response
        finally:
            with self._state_lock:
                self._callbacks.pop(callback_id, None)

    def _finish_callback(self, invocation: Invocation, response: Optional[JSON]) -> None:
        if invocation.cancel.is_set():
            self.send_canceled(invocation.invocation_id)
        elif response is None:
            self.send_error(invocation.invocation_id, -32001, "host callback did not respond")
        elif "error" in response:
            self.send_error(invocation.invocation_id, -32002, "host callback failed", response["error"])
        else:
            self.send_result(invocation.invocation_id, {"callback_result": response.get("result")})

    def _duplicate_callback(self, invocation: Invocation, input_value: JSON) -> None:
        """Send a deliberately reused m:* id; the broker must fail closed."""
        callback_id = self.next_callback_id(invocation.invocation_id)
        request = {
            "jsonrpc": "2.0",
            "id": callback_id,
            "method": self._callback_method(invocation),
            "params": self._callback_params(invocation, input_value),
        }
        self.send(request)
        self.send(request)

    def _terminal_during_callback(self, invocation: Invocation, input_value: JSON) -> None:
        """Violate causality by returning while a host callback is unresolved."""
        callback_id = self.next_callback_id(invocation.invocation_id)
        self.send({
            "jsonrpc": "2.0",
            "id": callback_id,
            "method": self._callback_method(invocation),
            "params": self._callback_params(
                invocation,
                {
                    "callback_input": input_value.get(
                        "callback_input",
                        {"op": "echo", "value": "late effect", "delay_ms": 200},
                    )
                },
            ),
        })
        self.send_result(invocation.invocation_id, {"returned_too_early": True})

    def _wrong_callback_direction(self, invocation: Invocation, input_value: JSON) -> None:
        """Pretend a module callback is a host request; broker must reset."""
        parts = invocation.invocation_id.split(":", 2)
        generation = parts[1] if len(parts) == 3 else "0"
        self.send({
            "jsonrpc": "2.0",
            "id": f"h:{generation}:999",
            "method": self._callback_method(invocation),
            "params": self._callback_params(invocation, input_value),
        })

    @staticmethod
    def _offset_invocation_id(invocation_id: str, offset: int) -> str:
        direction, generation, sequence = invocation_id.split(":", 2)
        return f"{direction}:{generation}:{int(sequence) + offset}"

    def progress(self, invocation_id: str, sequence: int, payload: Any) -> None:
        params = (
            {"invocation_id": invocation_id, "payload": {"seq": sequence, "value": payload}}
            if self._is_v3()
            else {"invocation_id": invocation_id, "seq": sequence, "payload": payload}
        )
        self.send({"jsonrpc": "2.0", "method": "module.progress", "params": params})

    def send_result(self, invocation_id: Any, result: Any) -> None:
        self.send({"jsonrpc": "2.0", "id": invocation_id, "result": {"pid": os.getpid(), "value": result}})

    def send_error(self, invocation_id: Any, code: int, message: str, data: Any = None) -> None:
        error: JSON = {"code": code, "message": message}
        if data is not None:
            error["data"] = data
        self.send({"jsonrpc": "2.0", "id": invocation_id, "error": error})

    def send_canceled(self, invocation_id: str) -> None:
        self.send_error(invocation_id, -32800, "canceled")


def main() -> int:
    worker = Worker()
    for raw_line in sys.stdin:
        try:
            frame = json.loads(raw_line)
        except json.JSONDecodeError:
            # The broker owns input validation.  A worker that receives a bad
            # frame does not invent a response id or corrupt another call.
            continue
        if isinstance(frame, dict):
            worker.receive(frame)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
