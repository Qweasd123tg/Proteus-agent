#!/usr/bin/env python3
"""Cold-start a relocated release in Xvfb; observe its real native→WASM→SSE path.

Uses no UI automation: the remembered project opens through normal launcher JS.
Personal preferences, provider credentials and the owner's running app are untouched.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time


def descendants(pid):
    result = []
    pending = [pid]
    while pending:
        parent = pending.pop()
        try:
            children = Path(f"/proc/{parent}/task/{parent}/children").read_text().split()
        except FileNotFoundError:
            continue
        for child in map(int, children):
            result.append(child)
            pending.append(child)
    return result


def stop(process):
    if process is None or process.poll() is not None:
        return
    owned = [process.pid, *descendants(process.pid)]
    groups = set()
    for pid in owned:
        try:
            groups.add(os.getpgid(pid))
        except ProcessLookupError:
            pass
    groups.discard(os.getpgrp())
    for group in groups:
        try:
            os.killpg(group, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        for group in groups:
            try:
                os.killpg(group, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.wait(timeout=5)


def main():
    desktop = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="proteus-native-smoke-") as temporary:
        root = Path(temporary)
        app = root / "relocated"
        shutil.copytree(desktop / "build/Proteus", app, symlinks=True)
        project = root / "project"
        project.mkdir()
        events = root / "events.jsonl"
        config = root / "fake.toml"
        config.write_text('''active_provider = "fake"
[providers.fake]
provider = "fake"
model = "fake-model"
[components.model]
command = "proteus-reference-worker"
[components.model.exports.model.fake]
[module_config.model.fake]
implementation = "fake"
[event_log]
path = ''' + json.dumps(str(events)) + "\n")
        settings = root / "settings/dev.proteus.agent/preferences.json"
        settings.parent.mkdir(parents=True)
        settings.write_text(json.dumps({"workspace": str(project), "config": str(config)}))
        display = application = None
        with (root / "native.log").open("w+") as log:
            try:
                display = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1440x1000x24", "-nolisten", "tcp"], stdout=subprocess.PIPE, stderr=log, text=True, start_new_session=True)
                display_number = display.stdout.readline().strip()
                assert display_number.isdecimal(), "Xvfb did not start"
                env = os.environ.copy()
                env.pop("WAYLAND_DISPLAY", None)
                env.pop("PROTEUS_CONFIG_PATH", None)
                # Xvfb has no GPU/compositor; the real desktop keeps normal rendering.
                env.update(DISPLAY=":" + display_number, GDK_BACKEND="x11", WEBKIT_DISABLE_COMPOSITING_MODE="1", NO_AT_BRIDGE="1", XDG_CONFIG_HOME=str(root / "settings"), XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"), PROTEUS_CONFIG_HOME=str(root / "proteus"))
                application = subprocess.Popen(["dbus-run-session", "--", str(app / "proteus-desktop")], cwd=project, env=env, stdout=log, stderr=log, start_new_session=True)
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    if application.poll() is not None:
                        raise AssertionError("Native application exited during startup")
                    if events.exists() and any("SessionStarted" in json.loads(line).get("event", {}) for line in events.read_text().splitlines()):
                        print("PASS: relocated native app → saved project → packaged backend → Leptos SSE SessionStarted")
                        return
                    time.sleep(0.1)
                raise AssertionError("Native client did not start its authenticated SSE session")
            except Exception:
                log.flush()
                log.seek(0)
                print(log.read()[-6000:])
                raise
            finally:
                stop(application)
                stop(display)


if __name__ == "__main__":
    main()
