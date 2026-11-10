"""Download the pinned public compatibility dataset outside the repository.
Usage: python3 scripts/collect-media-samples.py --destination /tmp/travel-gallery-real-media
The images/videos are test inputs, not shipped application assets.
"""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import urllib.parse
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--destination', required=True, type=Path)
args = parser.parse_args()
root = args.destination.expanduser().resolve()
project = Path(__file__).resolve().parent.parent
if root == project or project in root.parents:
    raise SystemExit('Choose a temporary fixture directory outside the repository.')
manifest = json.loads((project / 'docs/media-samples-manifest.json').read_text())
root.mkdir(parents=True, exist_ok=True)

def download(entry):
    relative = Path(entry['file'])
    if relative.is_absolute() or '..' in relative.parts:
        raise ValueError('Invalid fixture path')
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        url = urllib.parse.quote(entry['url'], safe=':/%')
        with urllib.request.urlopen(url, timeout=90) as response:
            data = response.read(128 * 1024 * 1024)
        if hashlib.sha256(data).hexdigest() != entry['sha256']:
            raise ValueError(f'Download hash mismatch: {relative}')
        with path.open('xb') as output:
            output.write(data)
    if hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']:
        raise ValueError(f'Existing fixture hash mismatch: {relative}')
    return relative

with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    for relative in pool.map(download, manifest):
        print(relative)
(root / 'manifest.json').write_text(json.dumps(manifest, indent=2))
for filename, data in [('bad-zero.jpg', b''), ('bad-header.jpg', b'deliberately invalid JPEG fixture')]:
    path = root / filename
    if path.exists() and path.read_bytes() != data:
        raise ValueError(f'Fixture collision: {path}')
    if not path.exists():
        path.write_bytes(data)
print(f'Validated {len(manifest)} public samples + 2 deliberate bad fixtures in {root}')
