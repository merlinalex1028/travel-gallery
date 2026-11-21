"""Exercise the two shipped macOS workers against pinned models, without the app."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import errno
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


ROOT = Path(__file__).resolve().parents[1]
BUNDLE = ROOT / 'src-tauri/target/release/bundle/macos/Travel Gallery.app/Contents/MacOS'
BUDGET = [sys.executable, '-B', str(ROOT / 'scripts/check-test-artifact-budget.py')]
SANDBOX = ['/usr/bin/sandbox-exec', '-p', '(version 1)(allow default)(deny network*)']
CASES = {
    'quality': ('face-landmarker-v1', 'prepare-eyes-model.mjs', 'qualityWorkerSha256'),
    'ai': ('siglip2-base224', 'prepare-ai-model.mjs', 'aiWorkerSha256'),
}


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def stamp(path):
    st = path.lstat()
    return (sha(path), st.st_size, st.st_mtime_ns, st.st_ino)


class HttpsRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        if not new_url.startswith('https://'):
            raise ValueError('Pinned model redirects must stay on HTTPS')
        return super().redirect_request(request, response, code, message, headers, new_url)


def download_weight(target, file, *, workers=6, block_bytes=16 * 1024**2, timeout=900):
    """Keep one owned file; resume an interrupted range at the last written byte."""
    assert file['url'].startswith('https://') and file['bytes'] > 0
    assert workers > 0 and block_bytes > 0 and timeout > 0
    opener = urllib.request.build_opener(HttpsRedirect())
    stop, lock = threading.Event(), threading.Lock()
    deadline = time.monotonic() + timeout
    completed = 0
    descriptor = os.open(target, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    succeeded = False

    def transfer(begin):
        nonlocal completed
        end = min(begin + block_bytes, file['bytes']) - 1
        offset = begin
        for attempt in range(3):
            if stop.is_set() or time.monotonic() >= deadline:
                raise TimeoutError('Pinned model transfer stopped')
            request = urllib.request.Request(file['url'], headers={'Range': f'bytes={offset}-{end}'})
            try:
                with opener.open(request, timeout=25) as response:
                    if (response.status != 206 or response.headers.get('Content-Range')
                            != f"bytes {offset}-{end}/{file['bytes']}"):
                        raise ValueError('Model server did not return the requested byte range')
                    while offset <= end:
                        if stop.is_set() or time.monotonic() >= deadline:
                            raise TimeoutError('Pinned model transfer stopped')
                        data = response.read(min(1024**2, end + 1 - offset))
                        if not data:
                            raise EOFError('Pinned model range ended early')
                        written = 0
                        while written < len(data):
                            count = os.pwrite(descriptor, data[written:], offset + written)
                            if count <= 0:
                                raise OSError('Model write made no progress')
                            written += count
                        offset += len(data)
                        with lock:
                            completed += len(data)
                    if response.read(1):
                        raise ValueError('Model response exceeded the requested byte range')
                return
            except (EOFError, TimeoutError, ConnectionError, http.client.IncompleteRead, urllib.error.URLError) as error:
                if (isinstance(error, urllib.error.HTTPError) and error.code not in (408, 500, 502, 503, 504)
                        or attempt == 2 or stop.is_set() or time.monotonic() >= deadline):
                    raise
        raise AssertionError('Unreachable range retry state')

    try:
        os.ftruncate(descriptor, file['bytes'])
        with ThreadPoolExecutor(max_workers=workers) as pool:
            futures = [pool.submit(transfer, begin) for begin in range(0, file['bytes'], block_bytes)]
            last_update = time.monotonic()
            try:
                while not all(future.done() for future in futures):
                    for future in futures:
                        if future.done() and future.exception():
                            raise future.exception()
                    if time.monotonic() - last_update >= 20:
                        print(f"{file['path']}: {completed} / {file['bytes']} bytes",
                              file=sys.stderr, flush=True)
                        last_update = time.monotonic()
                    time.sleep(.1)
                for future in futures:
                    future.result()
            finally:
                stop.set()
                for future in futures:
                    future.cancel()
        os.fsync(descriptor)
        if completed != file['bytes'] or sha(target) != file['sha256']:
            raise ValueError('Pinned model size or SHA-256 verification failed')
        succeeded = True
    finally:
        os.close(descriptor)
        if not succeeded:
            target.unlink()


def download_model(stage, model, prepare, profile):
    # The full downloader briefly hard-links its .part to the final name. For
    # the large weight in a private test directory, keep only one directory entry.
    for file in profile['files']:
        if file['bytes'] <= 512 * 1024**2:
            continue
        assert Path(file['path']).name == file['path']
        print(f"Downloading pinned {file['path']} ({file['bytes']} bytes)", file=sys.stderr, flush=True)
        download_weight(stage / file['path'], file)
    subprocess.run(['node', str(ROOT / 'scripts' / prepare), '--download', '--destination', str(stage)],
                   stdout=sys.stderr, check=True, timeout=120)
    print(f'Pinned {model} model verified', file=sys.stderr, flush=True)


def smoke(kind, release, existing_package):
    model, prepare, checksum = CASES[kind]
    profile = json.loads((ROOT / f'src-tauri/resources/ai/{model}.json').read_bytes())
    original = BUNDLE / f'travel-gallery-{kind}'
    assert sha(original) == release[checksum]
    weights = sum(file['bytes'] for file in profile['files'])
    largest_link = max(file['bytes'] for file in profile['files'] if file['bytes'] <= 512 * 1024**2)
    package_budget = ['--extra-root', str(existing_package)] if existing_package else []
    planned = original.stat().st_size + 1024**2 + (0 if existing_package else weights + largest_link)
    subprocess.run([*BUDGET, *package_budget, '--planned-source-files', '1', '--planned-files', '20',
                    '--planned-bytes', str(planned)],
                   stdout=sys.stderr, check=True)

    with tempfile.TemporaryDirectory(prefix=f'travel-gallery-{kind}-worker-qa-') as directory:
        run = Path(directory).resolve()
        package = existing_package or (run / 'model')
        cache, home = run / 'cache', run / 'home'
        for folder in (cache, home):
            folder.mkdir()
        copied = run / original.name
        shutil.copy2(original, copied)
        assert sha(copied) == release[checksum]
        if existing_package:
            subprocess.run(['node', str(ROOT / 'scripts' / prepare), '--verify', '--destination', str(package)],
                           stdout=sys.stderr, check=True, timeout=120)
        else:
            package.mkdir()
            download_model(package, model, prepare, profile)

        fixture = run / 'fixture.ppm'
        width = height = 256
        fixture.write_bytes(f'P6\n{width} {height}\n255\n'.encode() + bytes([72, 130, 192]) * (width * height))
        image = cache / f'{sha(fixture)}.jpg'
        subprocess.run(['/usr/bin/sips', '-s', 'format', 'jpeg', str(fixture), '--out', str(image)],
                       check=True, capture_output=True, timeout=15)
        model_sources = [package / file['path'] for file in profile['files']]
        sources = {path: stamp(path) for path in (fixture, image, package / 'travel-gallery-model.json', *model_sources)}
        env = {'PATH': '', 'HOME': str(home), 'CFFIXED_USER_HOME': str(home), 'TMPDIR': str(run)}
        control = subprocess.run([*SANDBOX, sys.executable, '-I', '-B', '-c',
                                  'import socket; socket.create_connection(("127.0.0.1", 9), 2)'],
                                 env=env, cwd=run, capture_output=True, text=True, timeout=8)
        assert control.returncode and f'[Errno {errno.EPERM}]' in control.stderr
        request = [dict(op='image', id='missing-preview', cacheFile='0' * 64 + '.jpg'),
                   dict(op='image', id='valid-preview', cacheFile=image.name)]
        if kind == 'ai':
            request.append(dict(op='text', id='valid-text', text='blue sky'))
        request.append(dict(op='shutdown'))
        worker = subprocess.run([*SANDBOX, str(copied), '--model-dir', str(package), '--cache-dir', str(cache)],
                                input=''.join(json.dumps(row) + '\n' for row in request), text=True,
                                env=env, cwd=run, capture_output=True, timeout=90)
        rows = list(map(json.loads, worker.stdout.splitlines()))
        assert worker.returncode == 0, f'{kind} worker exited {worker.returncode}'
        assert len(rows) == len(request)
        ready, rejected, inference, *extra = rows
        assert (ready['type'], ready['protocolVersion'], ready['profileId'], ready['dimension']) == (
            'ready', 1, profile['id'], profile['embeddingDimension'])
        assert rejected['type'] == 'error' and rejected['id'] == 'missing-preview'
        assert inference['type'] == 'result' and inference['id'] == 'valid-preview'
        result = dict()
        if kind == 'quality':
            assert ready['threadLimit'] == 1
            assert inference['hints'] == dict(width=width, height=height, faces=[], faceLimitReached=False)
            result.update(dimensions=[width, height], faceCount=0)
        else:
            assert ready['threadLimit'] == 2 and len(extra) == 1
            assert (extra[0]['type'], extra[0]['id']) == ('result', 'valid-text')
            for row in (inference, extra[0]):
                vector = row['vector']
                assert len(vector) == profile['embeddingDimension'] and all(map(math.isfinite, vector))
                assert math.isclose(math.hypot(*vector), 1, abs_tol=.0001)
            result.update(imageDimension=len(inference['vector']), textDimension=len(extra[0]['vector']), unitNorm=True)
        assert sources == {path: stamp(path) for path in sources}
        assert not list(home.iterdir()) and set(cache.iterdir()) == {image}
        subprocess.run([*BUDGET, *package_budget, '--extra-root', str(run)], stdout=sys.stderr, check=True)
    subprocess.run(BUDGET, stdout=sys.stderr, check=True)
    return dict(profileId=profile['id'], modelBytes=weights, executableSha256=release[checksum],
                emptyPath=True, loopbackControlErrno=errno.EPERM, missingPreviewError=rejected['code'],
                inference=result, shutdownExit=0, verifiedModelAndSourceUnchanged=True, temporaryDataRemoved=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worker', choices=CASES)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument('--download', action='store_true', help='Explicitly obtain the pinned QA resources')
    source.add_argument('--model-package', type=Path, help='Use an already prepared independent model package read-only')
    args = parser.parse_args()
    package = args.model_package
    if package:
        if not package.is_dir() or package.is_symlink():
            parser.error('--model-package must be an existing, non-symlink directory')
        package = package.resolve(strict=True)
        if package.is_relative_to(Path.home().resolve() / 'Library'):
            parser.error('Use an independent package outside the macOS Library directory')
    release = json.loads((ROOT / 'artifacts/build-info.json').read_bytes())
    print(json.dumps(dict(atUtc=datetime.now(timezone.utc).isoformat(), deliverySha256=release['sha256'],
                          worker=args.worker, **smoke(args.worker, release, package)), indent=2))


if __name__ == '__main__':
    def interrupted(number, frame):
        raise KeyboardInterrupt

    for item in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(item, interrupted)
    try:
        main()
    except KeyboardInterrupt:
        sys.exit('Interrupted; temporary worker resources removed.')
