#!/usr/bin/env python3
"""Run a selected Cargo test scope with an explicitly prepared process fixture."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
WORKER = "proteus-reference-worker"


def cargo_artifact(args, env):
    """Read Cargo's executable path; never guess an old binary from target/."""
    command = ["cargo", *args, "--message-format=json-render-diagnostics"]
    child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True)
    worker = None
    for line in child.stdout:
        message = json.loads(line)
        if message.get("reason") == "compiler-message":
            rendered = message["message"].get("rendered")
            if rendered:
                print(rendered, end="", file=sys.stderr)
        elif (message.get("reason") == "compiler-artifact"
              and message["target"]["name"] == WORKER
              and not message["profile"]["test"]
              and message.get("executable")):
            worker = message["executable"]
    status = child.wait()
    if status:
        raise SystemExit(status)
    return worker


def test_env():
    env = os.environ.copy()
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env.setdefault("RUST_TEST_THREADS", "4")
    env.setdefault("PYTHONDONTWRITEBYTECODE", "1")
    for key in ("NO_PROXY", "no_proxy"):
        entries = [value for value in env.get(key, "").split(",") if value]
        env[key] = ",".join(dict.fromkeys([*entries, "localhost", "127.0.0.1", "::1"]))
    return env


def selected_packages(args):
    packages = []
    for index, arg in enumerate(args):
        if arg in ("-p", "--package") and index + 1 < len(args):
            packages.append(args[index + 1])
        elif arg.startswith("--package="):
            packages.append(arg.split("=", 1)[1])
        elif arg.startswith("-p") and len(arg) > 2:
            packages.append(arg[2:])
    return packages


def main(args):
    if not args or args == ["--help"]:
        print("Usage: scripts/test.py full [Cargo options]\n"
              "       scripts/test.py -p PACKAGE [Cargo test options]\n\n"
              "Examples:\n"
              "  scripts/test.py -p proteus-core --test module_swap\n"
              "  scripts/test.py -p proteus-core --lib core::session_journal\n"
              "  scripts/test.py -p rg-search --lib\n"
              "  scripts/test.py full\n\n"
              "Build jobs and test threads respect CARGO_BUILD_JOBS/RUST_TEST_THREADS.")
        return 0 if args else 2
    full = args[0] == "full"
    if full:
        args = ["--workspace", "--no-fail-fast", *args[1:]]
    elif not selected_packages(args):
        raise SystemExit("Select -p PACKAGE, or use full for the complete workspace gate.")
    env = test_env()
    start = time.monotonic()
    if full:
        # Exact same selection/features/profile for compilation and execution.
        build_args = args[:args.index("--")] if "--" in args else args
        worker = cargo_artifact(["test", *build_args, "--no-run"], env)
        if not worker:
            raise SystemExit("Full gate did not build a reference worker executable.")
        env["PROTEUS_TEST_REFERENCE_WORKER"] = worker
    elif any(package.split("@", 1)[0] == "proteus-core" for package in selected_packages(args)):
        if env.get("PROTEUS_TEST_REFERENCE_WORKER"):
            worker = Path(env["PROTEUS_TEST_REFERENCE_WORKER"]).resolve()
            if not worker.is_file():
                raise SystemExit(f"Explicit reference worker is missing: {worker}")
            env["PROTEUS_TEST_REFERENCE_WORKER"] = str(worker)
        else:
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--format-version=1", "--no-deps"], cwd=ROOT, env=env, text=True))
            # A narrow fixture build must not rewrite the workspace feature cache.
            worker_dir = Path(metadata["target_directory"]) / "test-worker"
            worker = cargo_artifact(["build", "--locked", "-p", WORKER, "--bin", WORKER,
                                     "--target-dir", str(worker_dir)], env)
            if not worker:
                raise SystemExit("Reference worker build did not emit an executable.")
            env["PROTEUS_TEST_REFERENCE_WORKER"] = worker
    prepared = time.monotonic()
    print(f"Preparation: {prepared-start:.2f}s", file=sys.stderr, flush=True)
    status = subprocess.call(["cargo", "test", *args], cwd=ROOT, env=env)
    print(f"Test phase: {time.monotonic()-prepared:.2f}s; total: {time.monotonic()-start:.2f}s",
          file=sys.stderr)
    return status


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
