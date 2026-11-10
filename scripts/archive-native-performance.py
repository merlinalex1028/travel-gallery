"""Preserve a stopped, explicitly owned benchmark catalog before changing size."""
import argparse
import datetime
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--destination', type=Path, required=True)
parser.add_argument('--identifier', choices=['com.travelgallery.benchmark', 'com.travelgallery.offlineaudit'], default='com.travelgallery.benchmark')
parser.add_argument('--app-data', type=Path)
parser.add_argument('--app-cache', type=Path)
args = parser.parse_args()
destination = args.destination.absolute()
if destination.exists() or not destination.parent.is_dir():
    raise SystemExit('Archive destination must be new and have an existing parent.')
processes = subprocess.check_output(['ps', '-ax', '-o', 'comm='], text=True).splitlines()
if any(any(f'{name}.app/Contents/MacOS/' in process
           for name in ['Travel Gallery Bench', 'Travel Gallery Offline Bench'])
       for process in processes):
    raise SystemExit('Quit the benchmark application before archiving its data.')
roots = {
    'app-data': args.app_data or Path.home() / 'Library/Application Support' / args.identifier,
    'app-cache': args.app_cache or Path.home() / 'Library/Caches' / args.identifier,
}
markers = {}
for name, root in roots.items():
    if root.name != args.identifier or not root.absolute().is_relative_to(Path.home()):
        raise SystemExit('Only the explicitly named QA namespace under the user home can be archived.')
    if root.is_symlink() or not root.is_dir():
        raise SystemExit(f'Refusing absent or redirected benchmark directory: {root}')
    marker = json.loads((root / 'OWNED_NATIVE_BENCHMARK.json').read_text())
    if marker.get('identifier') != args.identifier or marker.get('count') not in [4, 5, 500, 30000, 50000]:
        raise SystemExit('Ownership marker does not match the benchmark namespace.')
    if root.stat().st_dev != destination.parent.stat().st_dev:
        raise SystemExit('Archive must be on the same filesystem for rename without copying.')
    markers[name] = marker
if markers['app-data'] != markers['app-cache']:
    raise SystemExit('Benchmark ownership markers disagree.')
destination.mkdir()
for name, root in roots.items():
    root.rename(destination / name)
(destination / 'ARCHIVE.json').write_text(json.dumps(dict(
    dateUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
    ownership=markers['app-data'],
    originalPaths={name: str(root) for name, root in roots.items()},
    method='rename only; no files deleted; normal desktop namespace is never a candidate',
), ensure_ascii=False, indent=2) + '\n')
print(destination)
