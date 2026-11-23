"""Type-check Windows publication/file-identity/power branches on a non-Windows host.
Requires: rustup target add x86_64-pc-windows-gnu
This does not link/run the app and does not replace Windows installation tests.
"""
import hashlib
import json
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from datetime import datetime, timezone

project = Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--allow-network',action='store_true')
args=parser.parse_args()
if args.allow_network:
    runtime=Path(os.environ['TMPDIR']).resolve().parent
    assert Path(os.environ['CARGO_HOME']).resolve().is_relative_to(runtime)
    assert Path(os.environ['CARGO_TARGET_DIR']).resolve().is_relative_to(runtime)
cargo = shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo')
toolchain_env = dict(os.environ, PATH=os.pathsep.join((str(Path(cargo).parent), os.environ.get('PATH', ''))))
lock = (project / 'src-tauri/Cargo.lock').read_text()
versions = {}
for block in lock.split('[[package]]'):
    name = re.search(r'^name = "([^"]+)"', block, re.M)
    version = re.search(r'^version = "([^"]+)"', block, re.M)
    if name and version:
        versions.setdefault(name[1], []).append(version[1])
cap = versions['cap-std'][0]
windows = next(v for v in versions['windows-sys'] if v.startswith('0.61.'))
source = (project / 'src-tauri/src/files.rs').read_text()
file_version = source[source.index('pub fn stamp('):source.index('pub fn source_path(')]
publish = project / 'src-tauri/src/publish.rs'
power = project / 'src-tauri/src/platform_power.rs'
errors = project / 'src-tauri/src/error.rs'
export_errors = project / 'src-tauri/src/export_errors.rs'
with tempfile.TemporaryDirectory(prefix='travel-gallery-winapi-') as folder:
    root = Path(folder)
    (root / 'src').mkdir()
    (root / 'Cargo.toml').write_text(f'''[package]
name="travel-gallery-windows-api-check"
version="0.0.0"
edition="2021"
[dependencies]
cap-std="={cap}"
windows-sys={{version="={windows}",features=["Win32_Foundation","Win32_Storage_FileSystem","Win32_System_Power"]}}
rusqlite="={versions['rusqlite'][0]}"
serde={{version="={versions['serde'][0]}",features=["derive"]}}
serde_json="={versions['serde_json'][0]}"
''')
    shutil.copyfile(project / 'src-tauri/Cargo.lock', root / 'Cargo.lock')
    modules = {'publish': publish, 'platform_power': power, 'error': errors, 'export_errors': export_errors}
    declarations = ''.join(f'#[path={json.dumps(str(path))}] pub mod {name};\n'
                           for name, path in modules.items())
    (root / 'src/lib.rs').write_text(declarations + '''
use crate::error::{AppError, Result};
use std::{fs::{self, File}, io, path::Path, time::UNIX_EPOCH};
''' + file_version)
    subprocess.run([cargo,'check',*([] if args.allow_network else ['--offline']),
        '--manifest-path',str(root/'Cargo.toml'),'--target','x86_64-pc-windows-gnu'],check=True,env=toolchain_env)
report = {
    'dateUtc': datetime.now(timezone.utc).isoformat(),
    'target': 'x86_64-pc-windows-gnu',
    'result': 'passed_type_check_only',
    'capStdVersion': cap,
    'windowsSysVersion': windows,
    'sourceSha256': {
        'src-tauri/src/publish.rs': hashlib.sha256(publish.read_bytes()).hexdigest(),
        'src-tauri/src/platform_power.rs': hashlib.sha256(power.read_bytes()).hexdigest(),
        'src-tauri/src/files.rs::fileVersionBlock': hashlib.sha256(file_version.encode()).hexdigest(),
        'src-tauri/src/error.rs': hashlib.sha256(errors.read_bytes()).hexdigest(),
        'src-tauri/src/export_errors.rs': hashlib.sha256(export_errors.read_bytes()).hexdigest(),
    },
    'limitations': ['No Windows process executed', 'Not the whole Tauri application build', 'No SQLite cross compilation', 'No installer, ACL, junction or runtime filesystem validation'],
}
(project / 'docs/windows-api-check.json').write_text(json.dumps(report, indent=2)+'\n')
print('Windows API branches type-check passed; runtime and installer validation remain pending.')
