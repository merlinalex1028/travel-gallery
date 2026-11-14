"""Verify the approved source-overlap candidate using only native picker grants."""
from contextlib import closing
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'candidate', Path(__file__).with_name('native-story-version-build-batch.py'))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)
batch, b, reuse, picker, exact = build.batch, build.b, build.reuse, build.picker, build.exact
BATCH = 'source-overlap'
b.APPROVAL = batch.ROOT / 'docs/native-source-overlap-budget.json'
b.RECEIPT = batch.ROOT / 'docs/native-source-overlap-latest.json'
STEPS = ('empty', 'parent', 'cancelled-warning', 'confirmed-child', 'duplicate-rejected',
         'marked', 'after-restart', 'reauthorized', 'finish')
MEDIA = {'owned-0.png', 'owned-1.png', 'owned-2.png', 'owned-3.png', 'owned-4.png'}
CHILD = {'owned-3.png', 'owned-4.png'}


def write_sources(source):
    reuse.write_sources(source)
    child = source / 'Nested'
    child.mkdir()
    for name in CHILD:
        (source / name).rename(child / name)


def snapshot(database, source):
    pid = picker.verified_process(database)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchall()
        roots = db.execute('SELECT id,absolute_path,is_online FROM source_roots ORDER BY absolute_path').fetchall()
        assets = db.execute("""SELECT a.id,a.source_id,a.relative_path,a.filename,a.file_state,r.rating,r.pick_state,r.revision
            FROM media_assets a JOIN asset_reviews r ON r.asset_id=a.id ORDER BY a.source_id,a.relative_path""").fetchall()
        scans = db.execute("""SELECT id,json_extract(checkpoint_json,'$.sourceId'),status,progress_done,
            progress_total,failure_count FROM jobs WHERE type='SCAN' ORDER BY id""").fetchall()
        jobs = db.execute('SELECT id,type,status,progress_done,failure_count FROM jobs ORDER BY id').fetchall()
        assert not any(job[2] in ('queued', 'running', 'paused') for job in jobs)
        assert len(roots) == db.execute('SELECT count(*) FROM source_options').fetchone()[0]
        assert len(assets) == db.execute('SELECT count(*) FROM media_assets').fetchone()[0]
    assert [path for _, path, _ in roots] == [str(source), str(source / 'Nested')][:len(roots)]
    for (sid, _, _), expected in zip(roots, (MEDIA, CHILD)):
        items = [(row[2], row[3]) for row in assets if row[1] == sid]
        assert len(items) == len(expected)
        assert set(items) == {(f'Nested/{name}' if expected == MEDIA and name in CHILD else name, name)
                              for name in expected}
    return dict(pid=pid, roots=roots, assets=assets, scans=scans, jobs=jobs)


def stable_identity(state):
    return ([row[:2] for row in state['roots']],
            [row[:4] + row[5:] for row in state['assets']])


def verify(step, state, prior):
    roots, assets, scans = state['roots'], state['assets'], state['scans']
    if step == 'empty':
        assert not roots and not assets and not state['jobs']
    elif step == 'parent':
        assert len(roots) == 1 and len(assets) == 5 and all(row[4] == 'available' for row in assets)
        assert any(row[1:] == (roots[0][0], 'completed', 5, 5, 0) for row in scans)
    elif step in ('cancelled-warning', 'duplicate-rejected'):
        assert state == prior['parent' if step == 'cancelled-warning' else 'confirmed-child']
    elif step == 'confirmed-child':
        parent = prior['parent']
        assert len(roots) == 2 and len(assets) == 7 and all(row[4] == 'available' for row in assets)
        assert roots[0] == parent['roots'][0]
        assert [row for row in assets if row[1] == roots[0][0]] == parent['assets']
        assert any(row[1:] == (roots[1][0], 'completed', 2, 2, 0) for row in scans)
    elif step == 'marked':
        assert len(roots) == 2 and len(assets) == 7
        assert sum(row[1] == roots[1][0] and row[5:7] == (4, 'keep') for row in assets) == 1
    else:
        assert stable_identity(state) == stable_identity(prior['marked'])
        child = roots[1][0]
        child_states = {row[4] for row in assets if row[1] == child}
        if step == 'after-restart':
            assert state['pid'] != prior['marked']['pid']
            assert all(not root[2] for root in roots) and {row[4] for row in assets} == {'offline'}
            assert scans == prior['marked']['scans']
        else:
            assert step == 'reauthorized' and state['pid'] == prior['after-restart']['pid']
            assert roots[1][2] and child_states == {'available'}
            assert any(row[1:] == (child, 'completed', 2, 2, 0) and row not in prior['marked']['scans']
                       for row in scans)


def run(approval):
    candidate, fixture, release, env = build.build_candidate(
        approval, BATCH, 'Overlap', write_sources, picker_only=True)
    source = fixture / 'source'
    preparation = json.loads((b.APP_PATH.parent / 'preparation.json').read_text())
    assert preparation['pickerOnly']
    assert not any('absolute-path' in name for name in preparation['parentEntitlements'])
    control = b.WORK / 'control'
    control.mkdir()
    database = b.DATA / 'catalog.sqlite'
    b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(database),
                   controlDirectory=str(control), pickerOnly=True)
    b.event('readyForNativeGui', source=str(source), **b.check())
    prior = {}
    for step in STEPS:
        deadline = time.monotonic() + 600
        request = control / 'step.json'
        while not request.is_file():
            b.check()
            if time.monotonic() >= deadline:
                raise TimeoutError(f'No native checkpoint: {step}')
            time.sleep(.25)
        observed = json.loads(request.read_text())
        assert observed.pop('step') == step
        if step == 'cancelled-warning':
            assert observed == {'warningSources': 1, 'warningIndexedRecords': 2, 'cancelClosed': True}
        elif step == 'reauthorized':
            assert observed == {'pickerMatched': 2, 'pickerConflicts': 0, 'pickerMissing': 0}
        else:
            assert not observed
        request.unlink()
        assert reuse.normal_state() == b.STATE['normalBefore']
        assert reuse.source_state() == b.STATE['sourceBefore']
        if step == 'finish':
            assert exact.exited(b.APP_PATH) and batch.verify_inputs() == b.STATE['sourceCommit']
            exact.publish(candidate, release, env)
            b.STATE.update(status='finished', qaQuitNormally=True)
            return
        state = snapshot(database, source)
        if step != 'empty' and step not in ('after-restart', 'reauthorized'):
            assert state['pid'] == prior['empty']['pid']
        verify(step, state, prior)
        prior[step] = state
        b.event('sourceOverlapCheckpoint', step=step, roots=len(state['roots']),
                assets=len(state['assets']), scans=len(state['scans']), **observed)


if __name__ == '__main__':
    batch.run = run
    assert b.APPROVAL.is_file() and not b.APPROVAL.is_symlink()
    with b.APPROVAL.open('r+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        approval = json.load(handle)
        assert (approval.get('batch') == BATCH and approval.get('status') == 'active'
                and approval.get('userAnswer') == '允许这一轮，结束即清理'
                and not approval.get('claimedAt') and approval['limits'] == batch.LIMITS
                and approval['requiredFreeBytes'] == batch.LIMITS['logicalBytes'])
        approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(approval, handle, ensure_ascii=False, indent=2)
        handle.truncate()
        handle.flush()
        raise SystemExit(batch.main(approval))
