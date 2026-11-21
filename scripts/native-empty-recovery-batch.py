"""Verify empty-source recovery with native pickers, reusing the delivered App."""
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
from threading import Timer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-empty-recovery-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-empty-recovery-latest.json'
reuse.SCOPE = ('Native empty-source change-folder recovery, successful replacement import, '
               'cleared picker state and additional search with no matches; no compilation')
reuse.RUN_PREFIX = 'travel-gallery-native-empty-recovery-'
reuse.PLANNED_BYTES = 512 * 1024**2
reuse.SOURCE_FILES = 7
STEPS = ('empty', 'populated', 'empty-source', 'replacement', 'filtered-empty', 'restored')
prior = {}
write_pngs = reuse.write_sources
base_main, base_save = reuse.main, b.save


def write_sources(source):
    populated = source / 'Populated'
    populated.mkdir()
    write_pngs(populated)
    (source / 'Empty').mkdir()
    replacement = source / 'Replacement'
    replacement.mkdir()
    for index in range(2):
        (replacement / f'replacement-{index}.png').write_bytes(
            (populated / f'owned-{index}.png').read_bytes())


def snapshot(label):
    assert label == STEPS[len(prior)], 'Native checkpoints must be sequential'
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    preparation = json.loads((b.WORK / 'qa/preparation.json').read_text())
    assert preparation['pickerOnly']
    assert not any('absolute-path' in key for key in preparation['parentEntitlements'])
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchall()
        roots = db.execute('SELECT id,absolute_path,is_online FROM source_roots ORDER BY absolute_path').fetchall()
        assets = db.execute('SELECT id,source_id,relative_path,file_state FROM media_assets ORDER BY id').fetchall()
        reviews = db.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        scans = db.execute("""SELECT json_extract(checkpoint_json,'$.sourceId'),status,
            progress_done,progress_total,failure_count FROM jobs WHERE type='SCAN'""").fetchall()
        assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        assert len(roots) == db.execute('SELECT count(*) FROM source_options').fetchone()[0]
    if label == 'empty':
        assert not roots and not assets and not reviews and not scans
        b.STATE['verifiedPid'] = pid
    else:
        assert pid == b.STATE['verifiedPid'], 'All recovery steps must use the same process'
        expected = {'Populated': 5}
        if label != 'populated':
            expected['Empty'] = 0
        if label in ('replacement', 'filtered-empty', 'restored'):
            expected['Replacement'] = 2
        source = (b.WORK / 'fixture/source').resolve(strict=True)
        assert {path for _, path, _ in roots} == {str(source / name) for name in expected}
        assert all(online for _, _, online in roots)
        for sid, path, _ in roots:
            count = expected[Path(path).name]
            items = [row for row in assets if row[1] == sid]
            assert len(items) == count and all(row[3] == 'available' for row in items)
            assert (sid, 'completed', count, count, 0) in scans
        assert len(reviews) == len(assets) == sum(expected.values())
        if label != 'populated':
            previous_assets, previous_reviews = prior['populated']
            assert all(row in assets for row in previous_assets)
            assert all(row in reviews for row in previous_reviews)
        if label in ('filtered-empty', 'restored'):
            assert (assets, reviews) == prior['replacement']
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    prior[label] = (assets, reviews)
    b.event('emptyRecoveryCheckpoint', label=label, roots=len(roots), assets=len(assets),
            scans=[dict(source=Path(path).name, status=status, done=done, total=total, failures=failures)
                   for sid, status, done, total, failures in scans
                   for root_id, path, _ in roots if root_id == sid],
            sameProcess=True, sourceAndFormalUnchanged=True)


def main():
    base_main()
    assert tuple(prior) == STEPS
    executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve(strict=True))
    processes = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True).splitlines()
    assert executable not in {line.strip() for line in processes}, 'Quit QA before finish'
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    b.STATE.update(qaQuitNormally=True, pickerOnly=True, savedScreenshots=0)


def save():
    if b.STATE.get('temporaryDirectoryRemoved') and b.WORK:
        for key in ('temporaryRoot', 'application', 'expectedDatabase', 'verifiedPid'):
            b.STATE.pop(key, None)
        for root in {str(b.WORK), str(b.WORK.resolve())}:
            cleaned = json.dumps(b.STATE).replace(root, '<cleaned run>')
            b.STATE.update(json.loads(cleaned))
    base_save()


reuse.write_sources, reuse.snapshot, reuse.main, b.save = write_sources, snapshot, main, save

if __name__ == '__main__':
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
