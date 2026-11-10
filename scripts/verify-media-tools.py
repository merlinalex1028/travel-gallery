"""Verify packaged local-only media tools against explicitly supplied QA videos."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--tools', type=Path, required=True)
parser.add_argument('--fixtures', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
assert not args.output.exists() and args.output.parent.is_dir()
tools = args.tools.resolve(strict=True)
fixtures = sorted(p for p in args.fixtures.resolve(strict=True).iterdir()
                  if p.suffix.lower() in ['.mov', '.mp4', '.mkv'])
assert fixtures and all(p.is_file() and not p.is_symlink() for p in fixtures)
env = {**os.environ, 'PATH': ''}


def run(name, *parameters):
    return subprocess.check_output([str(tools / name), *map(str, parameters)],
                                   env=env, stderr=subprocess.PIPE, timeout=30).decode()


def stamp(path):
    st = path.stat()
    return dict(bytes=st.st_size, mtimeNs=st.st_mtime_ns,
                sha256=hashlib.sha256(path.read_bytes()).hexdigest())


versions = {}
for name in ['ffmpeg', 'ffprobe']:
    versions[name] = run(name, '-version').splitlines()[0]
    assert 'version 9.0.2 ' in versions[name]
    assert re.search(r'GNU\s+Lesser\s+General\s+Public\s+License', run(name, '-L'))
    protocols = run(name, '-protocols')
    assert re.findall(r'^\s+([a-z][a-z0-9_]*)\s*$', protocols, re.M) == ['file', 'file'], protocols
    # Network support is compiled out; this fails before opening any socket.
    denied = subprocess.run([str(tools / name), '-v', 'error', '-i', 'https://127.0.0.1:9/blocked'],
                            capture_output=True, env=env, timeout=10)
    assert denied.returncode and b'Protocol not found' in denied.stderr, denied.stderr

results = []
with tempfile.TemporaryDirectory(prefix='travel-gallery-media-tool-qa-') as temporary:
    for index, source in enumerate(fixtures):
        before = stamp(source)
        metadata = json.loads(run('ffprobe', '-v', 'error', '-show_streams', '-show_format', '-of', 'json', source))
        stream = next(s for s in metadata['streams'] if s['codec_type'] == 'video')
        assert stream['width'] > 0 and stream['height'] > 0
        assert float(metadata['format']['duration']) > 0
        poster = Path(temporary) / f'{index}.jpg'
        run('ffmpeg', '-v', 'error', '-nostdin', '-ss', '0', '-i', source,
            '-frames:v', '1', '-vf', 'scale=256:256:force_original_aspect_ratio=decrease', '-y', poster)
        image = json.loads(run('ffprobe', '-v', 'error', '-show_streams', '-of', 'json', poster))['streams'][0]
        assert image['codec_name'] == 'mjpeg' and 0 < max(image['width'], image['height']) <= 256
        assert stamp(source) == before
        results.append(dict(file=source.name, source=before, codec=stream['codec_name'],
                            videoDimensions=[stream['width'], stream['height']],
                            posterDimensions=[image['width'], image['height']],
                            duration=metadata['format']['duration']))
report = dict(scope='Packaged CLI metadata and JPEG poster extraction with empty PATH; independent of WebView playback.',
              tools=str(tools), versions=versions, toolChecksums={n: stamp(tools / n)['sha256'] for n in versions},
              onlyFileProtocol=True, networkProtocolRejected=True, emptyPath=True,
              sourcesSha256SizeNanosecondMtimeUnchanged=True, results=results)
args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(report, ensure_ascii=False))
