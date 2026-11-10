"""Read-only verification of an explicitly owned native performance fixture."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import sqlite3
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--fixture', type=Path, required=True)
parser.add_argument('--catalog', type=Path, required=True)
parser.add_argument('--cache', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--review-ledger', type=Path, help='Expected native review inputs in delivery order')
parser.add_argument('--trace', type=Path, help='Independent observer JSONL; required with --review-ledger')
args = parser.parse_args()
assert bool(args.review_ledger) == bool(args.trace)
assert not args.output.exists() and args.output.parent.is_dir()
marker = json.loads((args.fixture / 'OWNED_NATIVE_BENCHMARK.json').read_text())
catalog_marker = json.loads((args.catalog.parent / 'OWNED_NATIVE_BENCHMARK.json').read_text())
assert marker == catalog_marker
assert marker['identifier'] in ['com.travelgallery.benchmark', 'com.travelgallery.offlineaudit'] and marker['count'] in [4, 5, 500, 30000, 50000]
assert not args.catalog.is_symlink() and not args.cache.is_symlink()
source = Path(marker['fixtureRoot']).resolve(strict=True)
manifest = json.loads((args.fixture / 'source-manifest.json').read_text())
assert len(manifest) == marker['count']
started = time.monotonic()
db = sqlite3.connect(args.catalog.resolve().as_uri() + '?mode=ro', uri=True)
rows = {row[0]: row[1:] for row in db.execute('SELECT id,relative_path,byte_size,mtime_ms FROM media_assets')}
assert len(rows) == len(manifest)
review_operations = 0
if args.review_ledger:
    baseline = sqlite3.connect((args.fixture / 'data/catalog.sqlite').resolve().as_uri() + '?mode=ro', uri=True)
    columns = 'asset_id,pick_state,rating,is_favorite,note,revision'
    expected_reviews = {r[0]: list(r[1:]) for r in baseline.execute(f'SELECT {columns} FROM asset_reviews')}
    by_name = dict(baseline.execute('SELECT filename,id FROM media_assets'))
    assert len(by_name) == len(expected_reviews) == marker['count'], 'Fixture filenames must be unique.'
    assert all(row == ['unreviewed', 0, 0, '', 0] for row in expected_reviews.values())
    ledger = json.loads(args.review_ledger.read_text())
    trace_rows = [json.loads(line) for line in args.trace.read_bytes().splitlines()]
    starts = [row for row in trace_rows if row.get('type') == 'nativeStart']
    assert len(starts) == 1 and all(row.get('runId') == starts[0]['runId'] for row in trace_rows)
    delivered = [row['event'] for row in trace_rows
                 if row.get('type') == 'frontendBatchEvent' and row['event']['type'] == 'keyDelivered']
    assert len(delivered) == len(ledger), 'Every delivered review key must be accounted for.'
    touched = set()
    for index, (entry, event) in enumerate(zip(ledger, delivered), 1):
        assert event['id'] == index and not event['repeat']
        assert (event['filename'], event['key']) == (entry['filename'], entry['key'])
        aid = by_name[entry['filename']]
        current = expected_reviews[aid]
        assert event['before'] == dict(pick=current[0], rating=current[1], favorite=bool(current[2]))
        key = entry['key']
        if key in ['p', 'u', 'x']:
            value = {'p': 'keep', 'u': 'maybe', 'x': 'reject'}[key]
            assert current[0] != value, 'No-op inputs must not count as mutations.'
            current[0] = value
        elif key == 'f':
            current[2] = 1 - current[2]
        else:
            assert key in '012345' and len(key) == 1 and current[1] != int(key)
            current[1] = int(key)
        current[4] += 1
        touched.add(aid)
    actual_reviews = {r[0]: list(r[1:]) for r in db.execute(f'SELECT {columns} FROM asset_reviews')}
    assert actual_reviews == expected_reviews, 'Persisted review state/revisions differ from actual inputs.'
    assert len(touched) == marker['count'], 'The review run must cover every fixture asset.'
    # Human relationships are outside the keyboard review operation.
    for table in ['asset_tags', 'asset_links', 'trip_assets']:
        if baseline.execute('SELECT 1 FROM sqlite_master WHERE type=\'table\' AND name=?', (table,)).fetchone():
            assert sorted(baseline.execute(f'SELECT * FROM {table}').fetchall()) == sorted(db.execute(f'SELECT * FROM {table}').fetchall())
    baseline.close()
    review_operations = len(ledger)
else:
    assert db.execute("SELECT count(*) FROM asset_reviews WHERE pick_state<>'unreviewed' OR rating<>0 OR is_favorite<>0 OR note<>'' OR revision<>0").fetchone()[0] == 0
assert db.execute('SELECT count(*) FROM asset_reviews').fetchone()[0] == marker['count']
assert not db.execute('PRAGMA foreign_key_check').fetchall()
assert db.execute('PRAGMA quick_check').fetchone()[0] == 'ok'
total_bytes = 0
for index, expected in enumerate(manifest):
    path = source / expected['relativePath']
    assert not path.is_symlink() and path.resolve(strict=True).is_relative_to(source)
    before = path.stat()
    assert (before.st_size, before.st_mtime_ns) == (expected['bytes'], expected['mtimeNs'])
    assert rows[expected['id']] == (expected['relativePath'], expected['bytes'], expected['mtimeNs'] // 1_000_000)
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    assert digest.hexdigest() == expected['sha256']
    after = path.stat()
    assert (after.st_size, after.st_mtime_ns) == (before.st_size, before.st_mtime_ns)
    total_bytes += after.st_size
    if (index + 1) % 5000 == 0:
        print(json.dumps(dict(verified=index + 1, total=len(manifest))), flush=True)
cache_rows = db.execute('SELECT cache_rel_path FROM thumbnail_cache').fetchall()
for relative, in cache_rows:
    assert Path(relative).name == relative
    path = args.cache / relative
    assert not path.is_symlink() and path.is_file()
result = dict(dateUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
              count=len(manifest), bytes=total_bytes,
              sourceSha256SizeAndOriginalNanosecondMtimeUnchanged=True,
              humanReviewChanges=marker['count'] if review_operations else 0,
              intendedReviewOperationsVerified=review_operations,
              unexpectedReviewChanges=0,
              referencedCacheFilesPresent=len(cache_rows),
              databaseQuickAndForeignKeyChecks='ok',
              elapsedSeconds=time.monotonic() - started,
              fixture=str(args.fixture.resolve()), catalog=str(args.catalog.resolve()),
              scope='All owned source files and review rows; independent read-only verification after native app exit.')
args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(result, ensure_ascii=False), flush=True)
