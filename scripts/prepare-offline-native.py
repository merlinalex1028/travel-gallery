"""Build a new macOS QA copy whose launcher denies network to the app process.

Does not start/stop apps, change network settings, or touch catalog/source data.
WebKit XPC service inheritance must be investigated separately; this is not an
assertion that every OS service is network isolated.
"""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--frozen', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
assert sys.platform == 'darwin'
frozen = args.frozen.resolve(strict=True)
manifest = json.loads((frozen / 'manifest.json').read_text())
source_app = frozen / 'Travel Gallery Bench.app'
source_binary = source_app / 'Contents/MacOS/travel-gallery'
assert hashlib.sha256(source_binary.read_bytes()).hexdigest() == manifest['binarySha256']
output = args.output.absolute()
assert not output.exists() and output.parent.is_dir()
output.mkdir()
app = output / 'Travel Gallery Offline Bench.app'
subprocess.run(['ditto', str(source_app), str(app)], check=True)
contents = app / 'Contents'
plist_path = contents / 'Info.plist'
with plist_path.open('rb') as stream:
    info = plistlib.load(stream)
assert info['CFBundleIdentifier'] == 'com.travelgallery.benchmark'
info['CFBundleIdentifier'] = 'com.travelgallery.offlineaudit'
info['CFBundleName'] = 'Travel Gallery Offline Bench'
info['CFBundleDisplayName'] = 'Travel Gallery Offline Bench'
with plist_path.open('wb') as stream:
    plistlib.dump(info, stream)
macos = contents / 'MacOS'
(macos / 'travel-gallery').rename(macos / 'travel-gallery-offline-child')
profile = contents / 'Resources/offline-deny-network.sb'
profile.write_text('(version 1)\n(allow default)\n(deny network*)\n')
source = Path(__file__).resolve().with_name('native-offline-launcher.c')
shutil.copy2(source, contents / 'Resources/native-offline-launcher.c')
command = ['clang', '-Wall', '-Wextra', '-Werror', '-Os', '-mmacosx-version-min=13.0',
           str(source), '-o', str(macos / 'travel-gallery')]
subprocess.run(command, check=True)
executables = ['travel-gallery-offline-child', 'travel-gallery-ai', 'travel-gallery-quality', 'ffmpeg', 'ffprobe']
for name in executables:
    subprocess.run(['codesign', '--force', '--sign', '-', '--timestamp=none', str(macos / name)],
                   check=True, capture_output=True)
subprocess.run(['codesign', '--force', '--sign', '-', '--timestamp=none', str(app)], check=True, capture_output=True)
subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True, capture_output=True)
assert hashlib.sha256(source_binary.read_bytes()).hexdigest() == manifest['binarySha256']
executables.append('travel-gallery')
report = dict(dateUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
              application=str(app), sourceFrozenApp=str(source_app),
              sourceBinarySha256=manifest['binarySha256'], launcherBuildCommand=command,
              launcherSourceSha256=hashlib.sha256(source.read_bytes()).hexdigest(),
              profile=profile.read_text(), signatureVerification='ad hoc deep/strict passed',
              binarySha256={name: hashlib.sha256((macos / name).read_bytes()).hexdigest() for name in executables},
              state='prepared, not launched or accepted',
              namespace='Embedded Tauri config remains com.travelgallery.benchmark; must verify actual app-data location at launch.',
              limits=['Quit the regular benchmark app and preserve its owned data before launching this separate copy.',
                      'Backend process/exec children are intended to inherit denial; actual native operation not yet verified.',
                      'WebKit XPC service responsibility/inheritance and all app request auditing remain separate gates.',
                      'No network preference, firewall, Wi-Fi, normal app, or source file is modified.'])
(output / 'preparation.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(application=str(app), state=report['state']), ensure_ascii=False))
