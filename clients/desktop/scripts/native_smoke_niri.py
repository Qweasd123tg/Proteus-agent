"""Exercise real native windows on niri without changing the graphics renderer."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def windows(project, pid=None):
    entries = json.loads(subprocess.check_output(["niri", "msg", "--json", "windows"]))
    return [window for window in entries if window.get("app_id") == "proteus-desktop"
            and (window.get("pid") == pid if pid is not None
                 else str(project) in window.get("title", ""))]


def action(name, window, *arguments):
    subprocess.run(["niri", "msg", "action", name, "--id", str(window["id"]), *arguments],
                   check=True, capture_output=True)


def wait_for(application, predicate):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        assert application.poll() is None, "Native application exited during window smoke"
        result = predicate()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError("Native window did not reach the expected state")


def exercise(application, project):
    with tempfile.TemporaryDirectory(prefix="proteus-smoke-input-") as temporary:
        socket = Path(temporary) / "input.sock"
        daemon = subprocess.Popen(["ydotoold", "--mouse-off", "--socket-path=" + str(socket)],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            wait_for(application, socket.exists)
            assert daemon.poll() is None, "ydotoold could not open /dev/uinput"
            # udev and the compositor must attach the newly created input device.
            time.sleep(2)
            exercise_windows(application, project, dict(os.environ, YDOTOOL_SOCKET=str(socket)))
        finally:
            daemon.terminate()
            daemon.wait(timeout=5)


def exercise_windows(application, project, input_env):
    pid = None

    def find(inspector):
        return next((window for window in windows(project, pid)
                     if window["title"].startswith("Proteus Inspector") == inspector), None)

    main = wait_for(application, lambda: find(False))
    pid = main["pid"]
    assert pid, "Compositor did not report the native process ID"
    for cycle in range(3):
        action("focus-window", main)
        wait_for(application, lambda: any(window["id"] == main["id"]
                                         and window["is_focused"] for window in windows(project, pid)))
        # The compositor reports focus before GTK has consumed the focus event.
        time.sleep(0.3)
        # Use physical key codes: synthetic Wayland keymaps can lose GTK accelerators.
        subprocess.run(["ydotool", "key", "29:1", "42:1", "23:1", "23:0", "42:0", "29:0"],
                       env=input_env, check=True)
        try:
            inspector = wait_for(application, lambda: find(True))
        except AssertionError:
            print("Native windows:", windows(project, pid), flush=True)
            raise
        assert len(windows(project, pid)) == 2, "Opening Inspector duplicated native windows"
        for width in [900, 1400, 1000]:
            action("set-window-width", inspector, str(width))
            time.sleep(0.5)
            assert application.poll() is None, "Native application crashed during Inspector resize"
        action("close-window", inspector)
        wait_for(application, lambda: find(True) is None)
        assert find(False), "Closing Inspector closed the chat"
        print(f"PASS: Inspector open/resize/close cycle {cycle + 1}", flush=True)
    action("close-window", main)
    assert application.wait(timeout=15) == 0, "Closing the chat did not exit cleanly"
    print("PASS: closing chat exits the native application", flush=True)
