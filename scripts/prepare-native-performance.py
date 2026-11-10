"""Prepare an isolated native QA catalog from a completed owned resource run.

macOS/APFS only. No network or deletion. Existing destinations are refused.
Source/cache files are independent APFS clones; the OS page cache is uncontrolled.
"""
import argparse
import ctypes
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


libc = ctypes.CDLL('/usr/lib/libSystem.B.dylib', use_errno=True)
libc.clonefile.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int]
libc.clonefile.restype = ctypes.c_int


def clone(source, destination):
    assert source.is_file() and not source.is_symlink()
    assert not destination.exists()
    destination.parent.mkdir(parents=True, exist_ok=True)
    if libc.clonefile(os.fsencode(source), os.fsencode(destination), 0):
        raise OSError(ctypes.get_errno(), 'APFS clone failed', str(destination))
    shutil.copystat(source, destination)
    before, after = source.stat(), destination.stat()
    assert before.st_size == after.st_size and before.st_mtime_ns == after.st_mtime_ns
    assert before.st_ino != after.st_ino and after.st_nlink == 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--from-run', type=Path, required=True)
    parser.add_argument('--count', type=int, required=True, choices=[4, 500, 30000, 50000])
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--install', action='store_true', help='Install only into absent com.travelgallery.benchmark directories')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        raise SystemExit('This fixture preparation requires macOS/APFS.')
    source_run = args.from_run.resolve(strict=True)
    output = args.output.absolute()
    if output.exists() or not output.parent.is_dir():
        raise SystemExit('Output must be a new directory with an existing parent.')
    owned = json.loads((source_run / 'OWNED_BENCHMARK.json').read_text())
    report = json.loads((source_run / 'report.json').read_text())
    assert report['files'] == owned['count'] and report['files'] >= args.count
    assert report['smallThumbnails'] == report['files']
    assert report['sourceSha256AndSizeAndNanosecondMtimeUnchanged'] is True
    assert report['humanReviewChanges'] == 0
    for name in ['initialScan', 'fullThumbnails', 'sha256Candidates', 'incrementalScan', 'cachedThumbnailPass']:
        assert report[name]['job']['status'] == 'completed' and report[name]['job']['failureCount'] == 0
    app_data = Path.home() / 'Library/Application Support/com.travelgallery.benchmark'
    app_cache = Path.home() / 'Library/Caches/com.travelgallery.benchmark'
    if args.install:
        processes = subprocess.check_output(['ps', '-ax', '-o', 'comm='], text=True).splitlines()
        assert not any('Travel Gallery Bench.app/Contents/MacOS/' in p for p in processes), 'Quit the benchmark app before installing.'
        assert not app_data.exists() and not app_cache.exists(), 'Existing benchmark app data/cache must be preserved, not overwritten.'
    output.mkdir()
    (output / 'data').mkdir()
    (output / 'cache').mkdir()
    fixtures = output / '来源 📷'
    fixtures.mkdir()
    original = sqlite3.connect((source_run / 'data/catalog.sqlite').as_uri() + '?mode=ro', uri=True)
    db = sqlite3.connect(output / 'data/catalog.sqlite')
    original.backup(db)
    original.close()
    db.execute('PRAGMA foreign_keys=ON')
    selected = db.execute('SELECT id FROM media_assets ORDER BY filename,id LIMIT ?', (args.count,)).fetchall()
    db.execute('CREATE TEMP TABLE retained(id TEXT PRIMARY KEY)')
    db.executemany('INSERT INTO retained VALUES(?)', selected)
    db.execute('DELETE FROM media_assets WHERE id NOT IN (SELECT id FROM retained)')
    # Remove resource workload memberships and expired cursors from this copy.
    for table in ['similar_groups', 'selection_snapshots', 'query_contexts', 'cache_access']:
        db.execute(f'DELETE FROM {table}')
    roots = dict(db.execute('SELECT id,absolute_path FROM source_roots'))
    assert len(roots) == 1
    manifest = []
    for aid, sid, relative, size, mtime, sha in db.execute('SELECT id,source_id,relative_path,byte_size,mtime_ms,sha256 FROM media_assets ORDER BY filename,id').fetchall():
        source = Path(roots[sid]) / relative
        destination = fixtures / relative
        clone(source, destination)
        stat = destination.stat()
        assert stat.st_size == size and stat.st_mtime_ns // 1_000_000 == mtime
        db.execute('UPDATE media_assets SET file_identity=? WHERE id=?', (f'{stat.st_dev}:{stat.st_ino}', aid))
        manifest.append(dict(id=aid, relativePath=relative, bytes=size, mtimeNs=stat.st_mtime_ns, sha256=sha))
    for relative, in db.execute('SELECT DISTINCT cache_rel_path FROM thumbnail_cache'):
        assert Path(relative).name == relative and relative.endswith('.jpg')
        clone(source_run / 'cache' / relative, output / 'cache' / relative)
    db.execute('UPDATE source_roots SET absolute_path=?', (str(fixtures),))
    assert db.execute('SELECT count(*) FROM media_assets').fetchone()[0] == args.count
    assert db.execute("SELECT count(*) FROM thumbnail_cache WHERE variant='small'").fetchone()[0] == args.count
    assert not db.execute('PRAGMA foreign_key_check').fetchall()
    assert db.execute('PRAGMA quick_check').fetchone()[0] == 'ok'
    db.commit()
    db.close()
    marker = dict(formatVersion=1, identifier='com.travelgallery.benchmark', count=args.count,
                  dateUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), fromRun=str(source_run),
                  fixtureRoot=str(fixtures), inputReportSha256=hashlib.sha256((source_run / 'report.json').read_bytes()).hexdigest(),
                  scope='Independent APFS file copies of a four-family public JPEG workload, warm small cache; not a diverse device corpus or cold HDD.')
    save(output / 'OWNED_NATIVE_BENCHMARK.json', marker)
    save(output / 'source-manifest.json', manifest)
    if args.install:
        app_data.mkdir()
        app_cache.mkdir()
        (app_cache / 'thumbs').mkdir()
        clone(output / 'data/catalog.sqlite', app_data / 'catalog.sqlite')
        for path in (output / 'cache').iterdir():
            clone(path, app_cache / 'thumbs' / path.name)
        save(app_data / 'OWNED_NATIVE_BENCHMARK.json', marker)
        save(app_cache / 'OWNED_NATIVE_BENCHMARK.json', marker)
    print(json.dumps(marker, ensure_ascii=False))


if __name__ == '__main__':
    main()
