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
    main = wait_for(application, lambda: next(iter(windows(project)), None))
    pid = main["pid"]
    assert pid, "Compositor did not report the native process ID"

    def assert_main_retained():
        assert application.poll() is None, "Native application exited during diagnostic shortcut smoke"
        current = windows(project, pid)
        assert len(current) == 1, f"Diagnostic shortcut changed native window count: {current}"
        assert current[0]["id"] == main["id"], "Diagnostic shortcut replaced the main window"

    assert_main_retained()
    # Canceling project selection must not exit the chat, and reopening must work.
    for cycle in range(2):
        action("focus-window", main)
        wait_for(application, lambda: any(window["id"] == main["id"]
                                         and window["is_focused"] for window in windows(project, pid)))
        time.sleep(0.3)
        subprocess.run(["ydotool", "key", "29:1", "42:1", "24:1", "24:0", "42:0", "29:0"],
                       env=input_env, check=True)
        chooser = wait_for(application, lambda: next((window for window in windows(project, pid)
                                                      if window["id"] != main["id"]), None))
        action("close-window", chooser)
        wait_for(application, lambda: len(windows(project, pid)) == 1)
        assert_main_retained()
        print(f"PASS: project chooser cancel/reopen cycle {cycle + 1} preserves the chat", flush=True)
    for cycle in range(3):
        action("focus-window", main)
        wait_for(application, lambda: any(window["id"] == main["id"]
                                         and window["is_focused"] for window in windows(project, pid)))
        # The compositor reports focus before GTK has consumed the focus event.
        time.sleep(0.3)
        # Use physical key codes: synthetic Wayland keymaps can lose GTK accelerators.
        subprocess.run(["ydotool", "key", "29:1", "42:1", "23:1", "23:0", "42:0", "29:0"],
                       env=input_env, check=True)
        # Diagnostics lives inside settings. niri cannot observe the embedded DOM;
        # this checks native survival/cardinality after physical shortcut delivery.
        for _ in range(10):
            time.sleep(0.1)
            assert_main_retained()
        for width in [900, 1400, 1000]:
            action("set-window-width", main, str(width))
            time.sleep(0.5)
            assert_main_retained()
        print(f"PASS: physical diagnostic shortcut/main-window resize cycle {cycle + 1} "
              "(embedded diagnostics DOM not observed)", flush=True)
    action("close-window", main)
    assert application.wait(timeout=15) == 0, "Closing the chat did not exit cleanly"
    print("PASS: closing chat exits the native application", flush=True)
