"""Seal a new, separately identified App Sandbox QA copy without network entitlements."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import plistlib
import subprocess
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--fixture', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--webkit-mach-lookup', action='store_true',
                    help='Diagnostic only: permit lookup of com.apple.nsurlsessiond, never network.client/server')
args = parser.parse_args()
assert sys.platform == 'darwin'
source = args.source.resolve(strict=True)
fixture = args.fixture.resolve(strict=True)
marker = json.loads((fixture / 'OWNED_NATIVE_BENCHMARK.json').read_text())
assert marker['identifier'] == 'com.travelgallery.offlineaudit'
assert type(marker['count']) is int and 1 <= marker['count'] <= 500
assert Path(marker['fixtureRoot']).resolve(strict=True).is_relative_to(fixture)
with (source / 'Contents/Info.plist').open('rb') as stream:
    info = plistlib.load(stream)
assert info['CFBundleIdentifier'] == marker['identifier'] and info['CFBundleExecutable'] == 'travel-gallery'
output = args.output.absolute()
assert not output.exists() and output.parent.is_dir()
output.mkdir()
app = output / 'Travel Gallery Offline Bench.app'
subprocess.run(['ditto', str(source), str(app)], check=True)
parent = {'com.apple.security.app-sandbox': True,
          'com.apple.security.files.user-selected.read-write': True,
          'com.apple.security.temporary-exception.files.absolute-path.read-write': [str(fixture) + '/']}
if args.webkit_mach_lookup:
    parent['com.apple.security.temporary-exception.mach-lookup.global-name'] = ['com.apple.nsurlsessiond']
child = {'com.apple.security.app-sandbox': True, 'com.apple.security.inherit': True}
for name, value in [('parent', parent), ('child', child)]:
    (output / f'{name}.entitlements.plist').write_bytes(plistlib.dumps(value))
helpers = ['travel-gallery-ai', 'travel-gallery-quality', 'ffmpeg', 'ffprobe']
macos = app / 'Contents/MacOS'
for name in helpers:
    subprocess.run(['codesign', '--force', '--sign', '-', '--timestamp=none', '--entitlements',
                    str(output / 'child.entitlements.plist'), str(macos / name)], check=True, capture_output=True)
subprocess.run(['codesign', '--force', '--sign', '-', '--timestamp=none', '--entitlements',
                str(output / 'parent.entitlements.plist'), str(app)], check=True, capture_output=True)
subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True, capture_output=True)
for name in ['travel-gallery', *helpers]:
    actual = plistlib.loads(subprocess.check_output(['codesign', '--display', '--entitlements', ':-', str(macos / name)], stderr=subprocess.PIPE))
    assert actual == (parent if name == 'travel-gallery' else child)
    assert not actual.get('com.apple.security.network.client') and not actual.get('com.apple.security.network.server')
report = dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), application=str(app),
              source=str(source), fixture=str(fixture), executableNameUnchanged=True,
              identifier=marker['identifier'], parentEntitlements=parent, childEntitlements=child,
              webkitMachLookupDiagnostic=args.webkit_mach_lookup,
              binarySha256={name: hashlib.sha256((macos / name).read_bytes()).hexdigest() for name in ['travel-gallery', *helpers]},
              signatureVerification='ad hoc deep/strict passed; parent/helper entitlement round-trip matched',
              state='prepared; actual process denial, GUI and XPC behavior still require execution',
              limits=['Only the owned fixture root has an absolute file exception; no global security/network settings changed.',
                      'An explicit nsurlsessiond Mach lookup exception is diagnostic only and may permit mediated network access; no WebKit isolation is inferred.',
                      'The offline-network-probe feature must reject startup unless the actual GUI process receives PermissionDenied on its local control.'])
(output / 'preparation.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(application=str(app), state=report['state']), ensure_ascii=False))
