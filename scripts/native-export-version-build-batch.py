"""Run the approved export regressions and one native candidate, then package verified output."""
from contextlib import closing
from datetime import datetime, timezone
import fcntl
import importlib.util
import csv
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET

sys.dont_write_bytecode = True


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


exact = load('native-exact-file-version-build-batch')
during = load('native-export-during-copy-batch')
batch, b, reuse, picker = exact.batch, exact.b, exact.reuse, exact.picker
ROOT, LIMITS, BUDGET = batch.ROOT, batch.LIMITS, batch.BUDGET
BATCH = 'export-source-version'
b.APPROVAL = ROOT / 'docs/native-export-version-budget.json'
b.RECEIPT = ROOT / 'docs/native-export-version-latest.json'
during.b, during.reuse = b, reuse
during.export.b, during.export.reuse, during.export.picker = b, reuse, picker
during.export.same_stamp = True
STEPS = ('empty', 'scanned', 'rewrite-before', 'before-checked', 'arm-copy', 'copy-checked', 'recovery', 'finish')


def plan_for(db, destination):
    rows = db.execute("""SELECT plan_json,consumed_by_job_id FROM export_plans
        WHERE json_extract(plan_json,'$.destination')=?""", (str(destination),)).fetchall()
    assert len(rows) == 1 and rows[0][1] is None
    plan = json.loads(rows[0][0])
    assert plan['policy'] == 'rename' and len(plan['items']) == 5
    assert all(item['sourceVersion'] and item['xmp'] and not item['skip'] for item in plan['items'])
    return plan


def verify_export(db, source, output, changed=None):
    jobs = db.execute("""SELECT id,status,progress_done,progress_total,failure_count FROM jobs
        WHERE type='EXPORT' AND json_extract(checkpoint_json,'$.destination')=?""", (str(output),)).fetchall()
    assert len(jobs) == 1 and jobs[0][1:] == ('completed', 5, 5, int(changed is not None)), jobs
    job = jobs[0][0]
    manifest = output / f'travel-gallery-{job}.csv'
    with manifest.open(encoding='utf-8-sig', newline='') as stream:
        report = {(row['type'], row['filename']): row for row in csv.DictReader(stream)}
    assert len(report) == 10
    expected_files = {manifest.name}
    for kind, table in (('media', 'export_run_items'), ('xmp', 'export_sidecar_items')):
        rows = db.execute(f"""SELECT asset_id,dest_relative_path,state,COALESCE(error_code,''),
            copied_bytes,COALESCE(verified_sha256,'') FROM {table} WHERE job_id=?""", (job,)).fetchall()
        assert len(rows) == 5
        for aid, name, status, code, size, sha in rows:
            row = report[kind, name]
            assert (row['assetId'],row['result'],row['errorCode'],row['sha256']) == (aid,status,code,sha)
            if changed and name == (changed if kind == 'media' else Path(changed).with_suffix('.xmp').name):
                assert (status,code) == (('failed','DB_CONFLICT') if kind == 'media' else ('skipped','MEDIA_EXPORT_FAILED'))
                assert size == 0 and not sha
            else:
                path = output / name
                assert (status,code) == ('copied','') and size == path.stat().st_size
                assert exact.sha(path) == sha
                if kind == 'media':
                    assert exact.sha(source / name) == sha
                else:
                    ET.parse(path)
                expected_files.add(name)
    assert {p.name for p in output.iterdir()} == expected_files


def accept(candidate, fixture, release, env):
    source = fixture / 'source'
    before_output, copy_output, recovered = (fixture / name for name in ('output', 'during-copy', 'recovered'))
    control = b.WORK / 'control'
    control.mkdir()
    database = b.DATA / 'catalog.sqlite'
    initial = b.STATE['sourceBefore']
    rewrite = None
    b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(database),
                   controlDirectory=str(control))
    b.event('readyForNativeGui', source=str(source), **b.check())
    try:
        for step in STEPS:
            request = control / 'step.json'
            deadline = time.monotonic() + 600
            while not request.is_file():
                b.check()
                if time.monotonic() >= deadline:
                    raise TimeoutError(f'No native checkpoint: {step}')
                time.sleep(.2)
            assert json.loads(request.read_text()) == {'step':step}
            request.unlink()
            assert reuse.normal_state() == b.STATE['normalBefore']
            if step == 'finish':
                assert exact.exited(b.APP_PATH) and reuse.source_state() == initial
                assert batch.verify_inputs() == b.STATE['sourceCommit']
                exact.publish(candidate, release, env)
                b.STATE.update(status='finished', qaQuitNormally=True)
                return
            if step in ('empty','scanned'):
                during.export.snapshot('before-import' if step == 'empty' else 'after-bootstrap')
                continue
            pid = picker.verified_process(database)
            assert pid == b.STATE['verifiedPid']
            with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True)) as db:
                db.execute('BEGIN')
                assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
                assert not db.execute('PRAGMA foreign_key_check').fetchall()
                if step == 'rewrite-before':
                    plan_for(db,before_output)
                    assert not any(before_output.iterdir())
                    assert reuse.source_state() == initial
                    rewrite = exact.rewrite(source/'owned-1.png',source/'owned-3.png')
                    b.event('sameStampRewriteAfterPreview', unchangedSizeMtimeAndInode=True)
                elif step == 'before-checked':
                    verify_export(db,source,before_output,'owned-1.png')
                    assert rewrite is not None
                    path,content,atime,mtime = rewrite
                    path.write_bytes(content)
                    os.utime(path,ns=(atime,mtime))
                    rewrite = None
                    assert reuse.source_state() == initial
                    b.event('changedPreviewItemRejected', mediaCopied=4, xmpCopied=4, csvRows=10)
                elif step == 'arm-copy':
                    plan_for(db,copy_output)
                    assert not any(copy_output.iterdir())
                    assert reuse.source_state() == initial
                elif step == 'copy-checked':
                    during.verify_result(db,source,copy_output,expected_failures=1)
                elif step == 'recovery':
                    verify_export(db,source,recovered)
                    assert reuse.source_state() == initial
                    b.event('rescanAndNewPreviewVerified', mediaCopied=5, xmpCopied=5, csvRows=10)
            if step == 'arm-copy':
                b.event('copyRaceArmed', sourceBytes=during.SOURCE_BYTES)
                capture = during.rewrite_during_copy(pid,source/during.NAME,copy_output,b.APP_PATH/exact.EXECUTABLE)
                b.STATE['duringCopy'] = capture
                b.event('sameStampRewriteDuringCopy', **capture)
    finally:
        if rewrite is not None:
            path,content,atime,mtime = rewrite
            path.write_bytes(content)
            os.utime(path,ns=(atime,mtime))


def run(approval):
    assert not any(path.exists() or path.is_symlink() for path in [*reuse.OWNED, *b.EXTRA, b.GENERATED])
    assert shutil.disk_usage(ROOT).free >= approval['requiredFreeBytes']
    subprocess.run(BUDGET + ['--approval-file',str(b.APPROVAL),'--planned-files',str(LIMITS['files']),
                    '--planned-bytes',str(LIMITS['logicalBytes']),'--planned-source-files',str(LIMITS['sourceFiles'])], check=True)
    release = json.loads((ROOT/'artifacts/build-info.json').read_text())
    exact.check_release(release,b.NORMAL)
    b.STATE.update(sourceCommit=batch.verify_inputs(), normalBefore=reuse.normal_state(), acceptedReleaseBefore=release['sha256'])
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-export-version-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.OWNS_GENERATED = True
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.save()
    cases = b.WORK/'cases'
    cases.mkdir()
    env = b.prepare_home()
    env.update(TMPDIR=str(cases), CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    b.command('fullRustRegressions',['node','scripts/native.mjs','cargo','test','--locked',
              '--manifest-path','src-tauri/Cargo.toml','--','--test-threads=1'],env)
    assert not any(cases.iterdir())
    shutil.rmtree(b.WORK/'target')
    candidate = b.build(False,env)
    b.STATE.update(normalCandidate=str(candidate),candidateExecutableSha256=exact.sha(candidate/exact.EXECUTABLE))
    fixture = b.WORK/'fixture'
    source = fixture/'source'
    source.mkdir(parents=True)
    during.write_sources(source)
    for name in ('during-copy','recovered'):
        (fixture/name).mkdir()
    b.STATE['sourceBefore'] = reuse.source_state()
    marker = dict(identifier=b.IDENTIFIER,count=5,fixtureRoot=str(source),scope=BATCH)
    (fixture/'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    b.APP_PATH = exact.seal_for_qa(candidate,'Export',fixture,env)
    b.OWNED_PATHS.extend(reuse.OWNED)
    accept(candidate,fixture,release,env)


if __name__ == '__main__':
    batch.run = run
    assert b.APPROVAL.is_file() and not b.APPROVAL.is_symlink()
    with b.APPROVAL.open('r+') as handle:
        fcntl.flock(handle,fcntl.LOCK_EX|fcntl.LOCK_NB)
        approval = json.load(handle)
        assert (approval.get('batch') == BATCH and approval.get('status') == 'active'
                and approval.get('userAnswer') == '允许这一轮，结束即清理'
                and not approval.get('claimedAt') and approval['limits'] == LIMITS
                and approval['requiredFreeBytes'] == LIMITS['logicalBytes'])
        approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(approval,handle,ensure_ascii=False,indent=2)
        handle.truncate()
        handle.flush()
        raise SystemExit(batch.main(approval))
