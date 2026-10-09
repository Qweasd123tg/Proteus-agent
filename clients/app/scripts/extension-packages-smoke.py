#!/usr/bin/env python3
"""Two actual Tauri processes, isolated data, real IPC and custom resource scheme."""
import os
from pathlib import Path
import subprocess
import tempfile

app = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='proteus-native-zip-', dir='/tmp/opencode') as folder:
    env = os.environ.copy()
    env.update(PROTEUS_EXTENSION_SMOKE_HOME=folder, XDG_DATA_HOME=folder+'/data', XDG_CONFIG_HOME=folder+'/config', XDG_CACHE_HOME=folder+'/cache')
    for phase in ('install', 'cold'):
        subprocess.run(['xvfb-run', '-a', 'cargo', 'run', '--locked', '--manifest-path', str(app/'src-tauri/Cargo.toml'), '--example', 'extension-packages-smoke', '--', phase], env=env, check=True, timeout=180)
