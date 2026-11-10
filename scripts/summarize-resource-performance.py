"""Validate a terminal owned resource run and summarize its raw process samples."""
import argparse
import collections
import datetime
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import subprocess


def rank(values, q):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * q) - 1)] if ordered else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    base = args.run.resolve(strict=True)
    status = json.loads((base / 'supervisor-status.json').read_text())
    assert status['returnCode'] == 0, 'A normal terminal result is required.'
    for pid in [status['childPid'], status['supervisorPid']]:
        result = subprocess.run(['ps', '-p', str(pid), '-o', 'comm='], capture_output=True, text=True)
        assert result.returncode != 0, f'Wait until the owned process has actually exited: {pid}'
    root = base / 'full-jpeg-50000'
    owned = json.loads((root / 'OWNED_BENCHMARK.json').read_text())
    report = json.loads((root / 'report.json').read_text())
    phase = json.loads((root / 'phase.json').read_text())
    assert owned['count'] == report['files'] == 50000 and phase['phase'] == 'completed'
    assert report['sourceSha256AndSizeAndNanosecondMtimeUnchanged'] is True
    assert report['humanReviewChanges'] == 0
    for name in ['initialScan', 'fullThumbnails', 'sha256Candidates', 'incrementalScan', 'cachedThumbnailPass']:
        job = report[name]['job']
        assert job['status'] == 'completed' and job['failureCount'] == 0
        assert job['progressDone'] == job['progressTotal'] == 50000
    db = sqlite3.connect((root / 'data/catalog.sqlite').as_uri() + '?mode=ro', uri=True)
    assert db.execute('PRAGMA quick_check').fetchone()[0] == 'ok'
    assert not db.execute('PRAGMA foreign_key_check').fetchall()
    assert db.execute('SELECT count(*) FROM media_assets').fetchone()[0] == 50000
    small = db.execute("SELECT count(*) FROM thumbnail_cache t JOIN media_assets a ON a.id=t.asset_id WHERE t.variant='small' AND t.source_mtime_ms IS a.mtime_ms AND t.source_byte_size=a.byte_size AND t.format_version=1").fetchone()[0]
    hashes = db.execute('SELECT count(*) FROM media_assets WHERE sha256 IS NOT NULL AND sha256_mtime_ms IS mtime_ms AND sha256_byte_size=byte_size').fetchone()[0]
    assert small == hashes == 50000
    families = [dict(sha256=sha, count=count) for sha, count in db.execute('SELECT sha256,count(*) FROM media_assets GROUP BY sha256 ORDER BY sha256')]
    assert len(families) == 4 and all(item['count'] == 12500 for item in families)
    groups = db.execute("SELECT g.id,count(*) FROM similar_groups g JOIN similar_group_items i ON i.group_id=g.id WHERE g.group_type='exact' GROUP BY g.id").fetchall()
    assert len(groups) == 4 and all(count == 12500 for _, count in groups)
    assert db.execute("SELECT count(*) FROM asset_reviews WHERE revision<>0 OR rating<>0 OR pick_state<>'unreviewed' OR is_favorite<>0 OR note<>''").fetchone()[0] == 0
    cache_rows = db.execute('SELECT DISTINCT cache_rel_path FROM thumbnail_cache').fetchall()
    cache_bytes = 0
    for relative, in cache_rows:
        assert Path(relative).name == relative and relative.endswith('.jpg')
        path = root / 'cache' / relative
        assert path.is_file() and path.stat().st_size > 0
        cache_bytes += path.stat().st_size
    assert cache_bytes == report['storage']['cache']['bytes']
    db.close()
    samples = [json.loads(line) for line in (base / 'process-samples.jsonl').read_text().splitlines()]
    assert len(samples) == status['sampleCount'] and samples
    stages = {}
    for name in sorted({row['phase'] for row in samples if row['phase']}):
        rows = [row for row in samples if row['phase'] == name]
        cpus = [row['cpuPercent'] for row in rows]
        stages[name] = dict(samples=len(rows), sampledCpuPercentP50=rank(cpus, .5),
            sampledCpuPercentP95=rank(cpus, .95), sampledCpuPercentMax=max(cpus),
            sampledRssMiBMax=max(row['rssKiB'] for row in rows) / 1024,
            firstSeconds=rows[0]['seconds'], lastSeconds=rows[-1]['seconds'])
    preview_checks = {}
    for name in ['initialScan', 'fullThumbnails', 'sha256Candidates', 'incrementalScan', 'cachedThumbnails']:
        path = root / (name + '-foreground.jsonl')
        rows = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        stage = report['cachedThumbnailPass' if name == 'cachedThumbnails' else name]
        cold = sum(not row['cacheEntryValidBefore'] for row in rows)
        cached = sum(row['cacheEntryValidBefore'] for row in rows)
        assert cold == stage['foregroundPreviewColdResolve']['samples']
        assert cached == stage['foregroundPreviewCachedResolve']['samples']
        assert cold + cached == stage['foregroundPreviewResolve']['samples']
        for flag, key in [(False, 'foregroundPreviewColdResolve'), (True, 'foregroundPreviewCachedResolve')]:
            values = [row['resolveMs'] for row in rows if row['cacheEntryValidBefore'] == flag]
            if values:
                assert abs(rank(values, .95) - stage[key]['p95Ms']) < 1e-9
        preview_checks[name] = dict(cold=cold, cached=cached, rawRows=len(rows))
    evidence_paths = [path for path in base.iterdir() if path.is_file() and path.name not in ['continuation.json', 'independent-summary.log']]
    evidence_paths += [path for path in root.iterdir() if path.is_file() and not path.name.startswith('.')]
    evidence_paths.append(Path(__file__).resolve())
    launch = json.loads((base / 'launch.json').read_text())
    for name in ['frozenSource', 'command']:
        path = Path(launch[name] if name == 'frozenSource' else launch['command'][0])
        if path not in evidence_paths:
            evidence_paths.append(path)
    evidence = {str(path): dict(bytes=path.stat().st_size, sha256=hashlib.sha256(path.read_bytes()).hexdigest()) for path in evidence_paths}
    result = dict(createdAt=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        status='completed-for-listed-workload', machine=launch['machine'], command=launch['command'],
        workload=report, independentDatabaseChecks=dict(quickCheck='ok', foreignKeyViolations=0,
            assets=50000, validSmallEntries=small, validSourceHashes=hashes, exactGroupMemberCounts=[count for _, count in groups],
            referencedCacheFilesPresent=len(cache_rows), humanReviewChanges=0, previewRawSamplesMatch=preview_checks),
        actualHashFamilies=families, postExitCacheLogicalBytes=cache_bytes,
        postExitDatabaseDirectoryBytes=sum(path.stat().st_size for path in (root / 'data').iterdir() if path.is_file()),
        resourceSamples=dict(intervalSeconds=.2, elapsedSeconds=status['elapsedSeconds'],
            percentileMethod='nearest rank: sorted[ceil(q*n)-1]', stages=stages,
            physicalFootprintSnapshots=json.loads((base / 'footprint-snapshots.json').read_text()),
            note='ps percentages are OS process samples; 100% is about one logical core. RSS is sampled; vmmap footprint/peak are different counters and retain raw units. Neither includes WebKit/GPU.'),
        acceptance=dict(fullFiftyThousandSmallThumbnailsPassed=True, sourceInvariantsPassed=True,
            nativeStartupProven=False, nativeGridFpsProven=False, nativeVisualKeyboardLatencyProven=False,
            externalHddProven=False), evidence=evidence)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(dict(status=result['status'], independentDatabaseChecks=result['independentDatabaseChecks'], stages=stages), ensure_ascii=False))


if __name__ == '__main__':
    main()
