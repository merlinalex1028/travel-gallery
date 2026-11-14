"""Build and publish a macOS candidate after isolated story export acceptance."""
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


exact = load('native-exact-file-version-build-batch')
story = load('native-story-version-batch')
batch, b, reuse, picker = exact.batch, exact.b, exact.reuse, exact.picker
ROOT, LIMITS, BUDGET = batch.ROOT, batch.LIMITS, batch.BUDGET
BATCH = 'story-source-version'
b.APPROVAL = ROOT / 'docs/native-story-version-build-budget.json'
b.RECEIPT = ROOT / 'docs/native-story-version-candidate.json'
story.b, story.reuse, story.picker = b, reuse, picker
story.export.b, story.export.reuse, story.export.picker = b, reuse, picker
story.export.same_stamp = True
STEPS = ('before-import', 'after-import', 'arm', 'verify-fixed', 'recovery', 'finish')


def accept(candidate, fixture, release, env):
    control = b.WORK / 'control'
    control.mkdir()
    b.STATE.update(status='ready', application=str(b.APP_PATH),
                   expectedDatabase=str(b.DATA / 'catalog.sqlite'), controlDirectory=str(control))
    b.event('readyForNativeGui', source=str(fixture / 'source'), **b.check())
    try:
        for step in STEPS:
            request = control / 'step.json'
            deadline = time.monotonic() + 600
            while not request.is_file():
                b.check()
                if time.monotonic() >= deadline:
                    raise TimeoutError(f'No native checkpoint: {step}')
                time.sleep(.25)
            assert json.loads(request.read_text()) == {'step': step}
            request.unlink()
            assert reuse.normal_state() == b.STATE['normalBefore']
            if step == 'finish':
                assert exact.exited(b.APP_PATH) and story.pending is None
                assert reuse.source_state() == b.STATE['sourceBefore']
                assert batch.verify_inputs() == b.STATE['sourceCommit']
                exact.publish(candidate, release, env)
                b.STATE.update(status='finished', qaQuitNormally=True)
                return
            story.snapshot(step)
    finally:
        story.restore()


def build_candidate(approval, batch_name, label, write_sources, *, picker_only=False):
    assert not any(p.exists() or p.is_symlink() for p in [*reuse.OWNED, *b.EXTRA, b.GENERATED])
    assert shutil.disk_usage(ROOT).free >= approval['requiredFreeBytes']
    subprocess.run(BUDGET + ['--approval-file', str(b.APPROVAL), '--planned-files', str(LIMITS['files']),
                            '--planned-bytes', str(LIMITS['logicalBytes']), '--planned-source-files',
                            str(LIMITS['sourceFiles'])], check=True)
    release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
    exact.check_release(release, b.NORMAL)
    b.STATE.update(sourceCommit=batch.verify_inputs(), normalBefore=reuse.normal_state(),
                   acceptedReleaseBefore=release['sha256'])
    b.TEMP = tempfile.TemporaryDirectory(prefix=f'travel-gallery-{batch_name}-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.OWNS_GENERATED = True
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.save()
    cases = b.WORK / 'cases'
    cases.mkdir()
    env = b.prepare_home()
    env.update(TMPDIR=str(cases), CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    b.command('fullRustRegressions', ['node', 'scripts/native.mjs', 'cargo', 'test', '--locked',
              '--manifest-path', 'src-tauri/Cargo.toml', '--', '--test-threads=1'], env)
    assert not any(cases.iterdir())
    shutil.rmtree(b.WORK / 'target')
    candidate = b.build(False, env)
    b.STATE.update(normalCandidate=str(candidate), candidateExecutableSha256=exact.sha(candidate / exact.EXECUTABLE))
    fixture = b.WORK / 'fixture'
    source = fixture / 'source'
    source.mkdir(parents=True)
    write_sources(source)
    b.STATE['sourceBefore'] = reuse.source_state()
    marker = dict(identifier=b.IDENTIFIER, count=len(b.STATE['sourceBefore']), fixtureRoot=str(source), scope=batch_name)
    (fixture / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    b.APP_PATH = exact.seal_for_qa(candidate, label, fixture, env, picker_only=picker_only)
    b.OWNED_PATHS.extend(reuse.OWNED)
    return candidate, fixture, release, env


def run(approval):
    candidate, fixture, release, env = build_candidate(approval, BATCH, 'Story', story.export.write_sources)
    accept(candidate, fixture, release, env)


if __name__ == '__main__':
    batch.run = run
    assert b.APPROVAL.is_file() and not b.APPROVAL.is_symlink()
    with b.APPROVAL.open('r+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        approval = json.load(handle)
        assert (approval.get('batch') == BATCH and approval.get('status') == 'active'
                and approval.get('userAnswer') == '允许这一轮，结束即清理'
                and not approval.get('claimedAt') and approval['limits'] == LIMITS
                and approval['requiredFreeBytes'] == LIMITS['logicalBytes'])
        approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(approval, handle, ensure_ascii=False, indent=2)
        handle.truncate()
        handle.flush()
        raise SystemExit(batch.main(approval))
