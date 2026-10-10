"""tgrep query execution; index/server lifecycle belongs to tgrep itself."""

from __future__ import annotations

import json
import subprocess
import tempfile
import threading
from typing import Any

from component_runtime import InvocationContext, ProtocolError

_active: set[subprocess.Popen[str]] = set()
_lock = threading.Lock()
TIMEOUT_SECONDS = 60


def stop(process: subprocess.Popen[str]) -> None:
    with _lock:
        if process.poll() is None:
            process.kill()
        process.wait()
        _active.discard(process)


def stop_all() -> None:
    with _lock:
        processes = list(_active)
    for process in processes:
        stop(process)


def validate_args(args: Any) -> dict[str, Any]:
    fields = {"query", "max_results", "use_case", "starts_with", "ends_with", "freshness"}
    if not isinstance(args, dict) or set(args) - fields:
        raise ProtocolError("search args must contain only declared arguments")
    if not isinstance(args.get("query"), str):
        raise ProtocolError("search requires string arg 'query'")
    limit = args.get("max_results", 20)
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 0:
        raise ProtocolError("max_results must be a non-negative integer")
    if args.get("freshness", "indexed") not in ("indexed", "current"):
        raise ProtocolError("freshness must be indexed or current")
    if args.get("use_case") is not None and not isinstance(args["use_case"], str):
        raise ProtocolError("use_case must be a string")
    for field in ("starts_with", "ends_with"):
        value = args.get(field, [])
        if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
            raise ProtocolError(f"{field} must be an array of strings")
    return {"query": args["query"], "max_results": limit,
            "freshness": args.get("freshness", "indexed"),
            "starts_with": args.get("starts_with", []),
            "ends_with": args.get("ends_with", [])}


def normalize_prefix(prefix: str) -> str:
    while prefix.startswith("./"):
        prefix = prefix[2:]
    return prefix


def matches_path(path: str, args: dict[str, Any]) -> bool:
    # Same literal prefix/suffix predicates as canonical SearchQuery.
    return (
        not args["starts_with"]
        or any(path.startswith(normalize_prefix(prefix)) for prefix in args["starts_with"])
    ) and (
        not args["ends_with"]
        or any(path.endswith(suffix) for suffix in args["ends_with"])
    )


def search(args: dict[str, Any], cwd: str, config: dict[str, str],
           context: InvocationContext) -> tuple[list[dict[str, Any]], bool]:
    context.ensure_active()
    if not args["query"].strip() or args["max_results"] == 0:
        return [], False
    command = [config.get("binary", "tgrep"), "--json", "--engine", "default",
               "--max-filesize", "1M"]
    if config.get("index_path"):
        command.extend(["--index-path", config["index_path"]])
    if args["freshness"] == "current":
        command.append("--no-index")
    # Search from cwd, retaining access to its index. Prefixes are predicates,
    # not necessarily existing directories; apply them before the global limit.
    command.extend(["--", args["query"], "."])
    chunks: list[dict[str, Any]] = []
    timed_out = threading.Event()
    with tempfile.TemporaryFile(mode="w+t", encoding="utf-8") as stderr:
        try:
            process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=stderr,
                                       text=True, encoding="utf-8", errors="replace")
        except FileNotFoundError as error:
            raise RuntimeError(f"cannot start tgrep: {error}; install tgrep or set binary") from error
        with _lock:
            _active.add(process)
        context.on_cancel(lambda: stop(process))

        def timeout() -> None:
            timed_out.set()
            stop(process)

        timer = threading.Timer(TIMEOUT_SECONDS, timeout)
        timer.start()
        limit_reached = False
        try:
            assert process.stdout is not None
            for line in process.stdout:
                context.ensure_active()
                event = json.loads(line)
                if event.get("type") != "match":
                    continue
                data = event["data"]
                path = data["path"]["text"].removeprefix("./")
                if not matches_path(path, args):
                    continue
                line_number = data["line_number"]
                if not isinstance(line_number, int):
                    raise RuntimeError("tgrep match has no integer line_number")
                chunks.append({"source": "process:tgrep_search", "path": path,
                               "content": data["lines"]["text"].rstrip("\r\n"),
                               "render_mode": "source_annotated", "score": None,
                               "metadata": {"line": line_number}})
                if len(chunks) >= args["max_results"]:
                    limit_reached = True
                    break
            if limit_reached:
                stop(process)
            else:
                status = process.wait()
                if status not in (0, 1):
                    context.ensure_active()
                    if timed_out.is_set():
                        raise RuntimeError("tgrep query timed out")
                    stderr.seek(0)
                    detail = stderr.read(4096).strip()
                    raise RuntimeError(f"tgrep exited with status {status}: {detail}")
            context.ensure_active()
            if timed_out.is_set():
                raise RuntimeError("tgrep query timed out")
            return chunks, limit_reached
        finally:
            timer.cancel()
            stop(process)
            process.stdout.close()
