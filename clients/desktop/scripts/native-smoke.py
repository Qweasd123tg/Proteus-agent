#!/usr/bin/env python3
"""Cold-start a relocated release; observe its real native→WASM→SSE path.

Default: headless Xvfb startup. --niri also exercises Inspector on the current GPU.
Personal preferences and provider credentials remain isolated in either mode.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time


def descendants(pid):
    result = set()
    pending = [pid]
    while pending:
        parent = pending.pop()
        try:
            children = set()
            for task in Path(f"/proc/{parent}/task").iterdir():
                try:
                    children.update(map(int, (task / "children").read_text().split()))
                except FileNotFoundError:
                    pass
        except FileNotFoundError:
            continue
        for child in children - result:
            result.add(child)
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--niri", action="store_true", help="Use current niri/Wayland with ydotool; open, resize and close Inspector")
    options = parser.parse_args()
    if options.niri:
        assert os.environ.get("WAYLAND_DISPLAY"), "--niri requires the current Wayland session"
        assert all(shutil.which(tool) for tool in ["niri", "ydotool", "ydotoold"]), "--niri requires niri, ydotool and ydotoold"
        assert os.access("/dev/uinput", os.W_OK), "--niri requires access to /dev/uinput"
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
                env = os.environ.copy()
                env.pop("PROTEUS_CONFIG_PATH", None)
                env.update(NO_AT_BRIDGE="1", XDG_CONFIG_HOME=str(root / "settings"), XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"), PROTEUS_CONFIG_HOME=str(root / "proteus"))
                if options.niri:
                    env["GDK_BACKEND"] = "wayland"
                else:
                    display = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1440x1000x24", "-nolisten", "tcp"], stdout=subprocess.PIPE, stderr=log, text=True, start_new_session=True)
                    display_number = display.stdout.readline().strip()
                    assert display_number.isdecimal(), "Xvfb did not start"
                    env.pop("WAYLAND_DISPLAY", None)
                    # Xvfb has no GPU/compositor; --niri keeps the real rendering path.
                    env.update(DISPLAY=":" + display_number, GDK_BACKEND="x11", WEBKIT_DISABLE_COMPOSITING_MODE="1")
                application = subprocess.Popen(["dbus-run-session", "--", str(app / "proteus-desktop")], cwd=project, env=env, stdout=log, stderr=log, start_new_session=True)
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    if application.poll() is not None:
                        raise AssertionError("Native application exited during startup")
                    if events.exists() and any("SessionStarted" in json.loads(line).get("event", {}) for line in events.read_text().splitlines()):
                        print("PASS: relocated native app → saved project → packaged backend → Leptos SSE SessionStarted")
                        if options.niri:
                            from native_smoke_niri import exercise
                            exercise(application, project)
                            log.flush()
                            log.seek(0)
                            assert "eglMakeCurrent failed" not in log.read(), "Native renderer reported EGL failures"
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
