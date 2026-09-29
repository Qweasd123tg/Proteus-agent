"""Click real Tauri chrome on the smoke test's private X display."""
import subprocess
import shutil
import time


def exercise(application, env):
    def xdo(*args, check=True):
        return subprocess.run(['xdotool', *map(str, args)], env=env, check=check,
                              capture_output=True, text=True, timeout=10).stdout.strip()

    def window(title):
        ids = xdo('search', '--onlyvisible', '--name', title, check=False).splitlines()
        return ids[-1] if ids else None

    def wait(check, message):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            assert application.poll() is None, 'Native app exited during titlebar check'
            value = check()
            if value:
                return value
            time.sleep(.1)
        if shutil.which('import'):
            subprocess.run(['import', '-window', 'root', '/tmp/proteus-native-chrome-failure.png'],
                           env=env, check=False, timeout=5)
        raise AssertionError(message)

    def click(wid, x, y):
        xdo('windowraise', wid)
        xdo('windowfocus', '--sync', wid)
        xdo('mousemove', '--window', wid, x, y, 'click', 1)

    def close(wid):
        geometry = dict(line.split('=', 1) for line in xdo('getwindowgeometry', '--shell', wid).splitlines())
        click(wid, int(geometry['WIDTH']) - 22, 20)

    def close_when_ready(wid, title):
        # A mapped WebKit window can still be compiling debug WASM. Retry the
        # same button while it loads; never send input to a different window.
        deadline = time.monotonic() + 45
        while window(title) and time.monotonic() < deadline:
            close(wid)
            time.sleep(1)
        wait(lambda: not window(title), 'Native close button failed: ' + title)

    main = wait(lambda: window('^Proteus — /'), 'Chat window missing')
    # SSE can start just before the desktop stylesheet/module finishes loading.
    time.sleep(1)
    click(main, 60, 74)
    click(main, 100, 118)
    launcher = wait(lambda: window('^Proteus — открыть проект$'), 'Sidebar project action failed')
    time.sleep(2)
    close_when_ready(launcher, '^Proteus — открыть проект$')
    click(main, 60, 74)
    # Focus starts at the first menu item; select Inspector with the real keyboard.
    time.sleep(.2)
    xdo('key', 'Down', 'Return')
    time.sleep(5)
    assert not window('^Proteus Inspector'), 'Diagnostics opened a separate window'
    # Main stays alive; capture the embedded native WebKit view for inspection.
    xdo('windowfocus', '--sync', main)
    xdo('key', 'ctrl+shift+i')
    time.sleep(25)
    assert not window('^Proteus Inspector'), 'Diagnostic shortcut opened a separate window'
    if shutil.which('import'):
        subprocess.run(['import','-window',main,'/tmp/proteus-native-modules.png'],env=env,check=True,timeout=5)
    close(main)
    assert application.wait(timeout=15) == 0, 'Chat close button did not exit cleanly'
    print('PASS: native titlebar project/embedded diagnostics actions and close semantics', flush=True)
