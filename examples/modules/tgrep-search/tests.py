"""Real tool protocol and indexed/scan search checks; no model/API required."""

from __future__ import annotations

import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import backend  # noqa: E402
from component_runtime import InvocationContext  # noqa: E402

BINARY = os.environ.get("TGREP_BINARY") or shutil.which("tgrep")
TOOL = Path(__file__).with_name("tool.py")


class Peer:
    def __init__(self, config):
        self.process = subprocess.Popen([sys.executable, "-B", str(TOOL)], text=True,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE)
        self.responses = queue.Queue()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()
        self.sequence = 0
        self.send({"id": "h:1:0", "method": "initialize", "params": {
            "protocol_version": "v3", "component_id": "tgrep-test", "exports": [{
                "slot": "tool", "module_id": "tgrep_search", "contract_version": "v5",
                "composition": "ordered_many", "host_features": [], "module_config": config,
            }],
        }})
        self.manifest = self.receive()

    def read(self):
        for line in self.process.stdout:
            self.responses.put(json.loads(line))

    def send(self, frame):
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", **frame}) + "\n")
        self.process.stdin.flush()

    def start(self, method, params):
        self.sequence += 1
        wire_id = f"h:1:{self.sequence}"
        self.send({"id": wire_id, "method": method, "params": {
            "export": {"slot": "tool", "module_id": "tgrep_search"},
            "lineage": {"root_invocation_id": wire_id, "parent_invocation_id": None, "depth": 0},
            "params": params,
        }})
        return wire_id

    def receive(self):
        return self.responses.get(timeout=10)

    def invoke(self, cwd, args):
        self.start("invoke", request(cwd, args))
        return self.receive()

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
        self.reader.join(timeout=2)
        self.process.stdout.close()
        self.process.stderr.close()


def request(cwd, args):
    return {"call": {"id": "test-call", "name": "search", "args": args,
                     "surface": "function", "raw_arguments": None},
            "cwd": str(cwd), "attribution": {"execution_id": "test-execution", "agent": None},
            "skills": {"disabled": []}}


class ProtocolTests(unittest.TestCase):
    def test_discovery_and_strict_config(self):
        peer = Peer({})
        self.addCleanup(peer.close)
        self.assertEqual(peer.manifest["result"]["exports"][0]["slot"], "tool")
        peer.start("list", None)
        spec = peer.receive()["result"]["result"][0]["spec"]
        self.assertEqual(spec["name"], "search")
        self.assertEqual(spec["safety"], "ReadOnly")
        self.assertTrue(spec["supports_parallel_tool_calls"])
        invalid = Peer({"unknown": True})
        self.addCleanup(invalid.close)
        self.assertIn("error", invalid.manifest)

    def test_invalid_arguments_and_missing_binary(self):
        peer = Peer({"binary": "/definitely/missing/tgrep"})
        self.addCleanup(peer.close)
        for args in ({"query": "needle", "max_results": -1},
                     {"query": "needle", "max_results": True},
                     {"query": "needle", "freshness": "unknown"},
                     {"query": "needle", "starts_with": "src/"},
                     {"query": "needle", "unknown": 1}):
            self.assertEqual(peer.invoke("/tmp", args)["error"]["code"], -32602)
        zero = peer.invoke("/tmp", {"query": "needle", "max_results": 0})
        self.assertEqual(zero["result"]["result"]["metadata"]["results"], 0)
        missing = peer.invoke("/tmp", {"query": "needle"})
        self.assertIn("cannot start tgrep", missing["error"]["message"])

    def test_cancel_stops_query_and_keeps_component_usable(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            marker = directory / "pid"
            binary = directory / "slow-search"
            binary.write_text(f"#!{sys.executable}\nimport os,time\n"
                              f"open({str(marker)!r},'w').write(str(os.getpid()))\ntime.sleep(120)\n")
            binary.chmod(0o700)
            peer = Peer({"binary": str(binary)})
            self.addCleanup(peer.close)
            wire_id = peer.start("invoke", request(directory, {"query": "needle"}))
            deadline = time.monotonic() + 5
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(marker.exists(), "query subprocess must actually start")
            peer.send({"method": "$/cancelRequest", "params": {
                "invocation_id": wire_id, "cause": "user",
            }})
            canceled = peer.receive()
            self.assertEqual(canceled["error"]["code"], -32800)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(marker.read_text()), 0)
            peer.start("list", None)
            self.assertIn("result", peer.receive())

    def test_query_timeout(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "slow-search"
            binary.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(120)\n")
            binary.chmod(0o700)
            context = InvocationContext(None, "h:1:1", {}, {})
            previous = backend.TIMEOUT_SECONDS
            backend.TIMEOUT_SECONDS = 0.1
            try:
                with self.assertRaisesRegex(RuntimeError, "timed out"):
                    backend.search(backend.validate_args({"query": "needle"}), directory,
                                   {"binary": str(binary)}, context)
            finally:
                backend.TIMEOUT_SECONDS = previous


@unittest.skipUnless(BINARY, "set TGREP_BINARY or install tgrep for real index tests")
class IndexedTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / "src").mkdir()
        (self.root / "src/main.rs").write_text("needle_one\nneedle_two\n")
        (self.root / "src:other.rs").write_text("needle_three\n")
        (self.root / "elsewhere.md").write_text("needle_four\n")
        (self.root / ".ignore").write_text("ignored.rs\n")
        (self.root / "ignored.rs").write_text("needle_ignored\n")
        subprocess.run([BINARY, "index", str(self.root), "--max-filesize", "1M"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.peer = Peer({"binary": BINARY})
        self.addCleanup(self.peer.close)

    def result(self, **args):
        response = self.peer.invoke(self.root, {"query": "needle", **args})
        self.assertNotIn("error", response)
        return response["result"]["result"]

    def test_indexed_regex_filters_limit_and_errors(self):
        result = self.result(query="needle_(one|three)", starts_with=["././src"], ends_with=[".rs"])
        self.assertEqual({c["path"] for c in result["metadata"]["chunks"]},
                         {"src/main.rs", "src:other.rs"})
        self.assertEqual(self.result(starts_with=["src/"], max_results=1)["metadata"]["results"], 1)
        self.assertEqual(self.result(query="absent")["output"], "(no matches)")
        self.assertIn("error", self.peer.invoke(self.root, {"query": "["}))
        self.assertNotIn("ignored.rs", {c["path"] for c in self.result()["metadata"]["chunks"]})

    def test_current_finds_add_edit_delete_after_index(self):
        (self.root / "added.rs").write_text("fresh_added\n")
        (self.root / "src/main.rs").write_text("fresh_edited\n")
        (self.root / "src:other.rs").unlink()
        fresh = self.result(query="fresh_", freshness="current")
        self.assertEqual({c["path"] for c in fresh["metadata"]["chunks"]},
                         {"added.rs", "src/main.rs"})
        old = self.result(query="needle_(one|three)", freshness="current")
        self.assertEqual(old["metadata"]["results"], 0)

    def test_scan_and_index_return_same_rows(self):
        indexed = self.result()["metadata"]["chunks"]
        current = self.result(freshness="current")["metadata"]["chunks"]
        self.assertEqual(sorted((c["path"], c["metadata"]["line"], c["content"]) for c in indexed),
                         sorted((c["path"], c["metadata"]["line"], c["content"]) for c in current))

    def test_server_refreshes_add_edit_delete(self):
        server = subprocess.Popen([BINARY, "serve", str(self.root), "--max-filesize", "1M",
                                   "--poll-interval", "1"],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(lambda: backend.stop(server))
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            status = subprocess.run([BINARY, "status", str(self.root)], capture_output=True,
                                    text=True, timeout=5)
            if "Server status" in status.stdout and "Last successful reconcile:" in status.stdout:
                break
            time.sleep(0.02)
        else:
            self.fail("tgrep server must complete startup reconciliation")
        # Warm cache before mutations, then verify that watcher refreshes it.
        self.result()
        (self.root / "added.rs").write_text("fresh_added\n")
        (self.root / "src/main.rs").write_text("fresh_edited\n")
        (self.root / "src:other.rs").unlink()
        expected = {"added.rs", "src/main.rs"}
        self.assertEqual({c["path"] for c in self.result(query="fresh_", freshness="current")
                          ["metadata"]["chunks"]}, expected)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            fresh = self.result(query="fresh_")["metadata"]["chunks"]
            old = self.result(query="needle_(one|three)")["metadata"]["chunks"]
            if {c["path"] for c in fresh} == expected and not old:
                return
            time.sleep(0.03)
        self.fail("indexed server results must converge after add/edit/delete")


if __name__ == "__main__":
    unittest.main()
