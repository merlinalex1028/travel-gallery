"""Small lifecycle regression: exports count toward total files, never source inputs."""
from pathlib import Path
import datetime
import json
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
from native_fixture_budget import source_inventory

root = Path(__file__).resolve().parent.parent
subprocess.run([sys.executable, str(root/'scripts/check-test-artifact-budget.py'),
    '--planned-source-files', '6', '--planned-files', '20', '--planned-bytes', str(1024**2)], check=True)
with tempfile.TemporaryDirectory(prefix='travel-gallery-native-counter-') as temporary:
    work = Path(temporary)
    fixture = work/'fixture'
    for folder in ('source', 'held-out', 'export-target'):
        (fixture/folder).mkdir(parents=True)
    for index in range(5):
        (fixture/'source'/f'owned-{index}.png').write_bytes(bytes([index])*70)
    (fixture/'source/owned-4.png').rename(fixture/'held-out/owned-4.png')
    for index in range(3):
        (fixture/'export-target'/f'owned-{index}.png').write_bytes(bytes([index])*70)
    source = source_inventory(fixture)
    assert len(source) == 5 and sum(p.stat().st_size for p in source) == 350
    assert len(list(fixture.rglob('owned-*.png'))) == 8
    (fixture/'source/owned-5.png').write_bytes(b'x'*70)
    assert len(source_inventory(fixture)) == 6
    (fixture/'source/owned-5.png').unlink()
    sentinel = work/'outside.txt'
    sentinel.write_bytes(b'protected')
    link = fixture/'source/link.png'
    link.symlink_to(sentinel)
    try:
        source_inventory(fixture)
    except RuntimeError:
        pass
    else:
        raise AssertionError('Source file link was followed')
    link.unlink()
    (fixture/'held-out/owned-4.png').rename(fixture/'source/owned-4.png')
    (fixture/'held-out').rmdir()
    (fixture/'held-out').symlink_to(work, target_is_directory=True)
    try:
        source_inventory(fixture)
    except RuntimeError:
        pass
    else:
        raise AssertionError('Source directory link was followed')
    assert sentinel.read_bytes() == b'protected'
    (fixture/'held-out').unlink()
    subprocess.run([sys.executable, str(root/'scripts/check-test-artifact-budget.py'), '--extra-root', temporary], check=True)
report = dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), passed=True,
    fiveInputsPlusThreeCopies=dict(sourceFiles=5, sourceBytes=350, allFixtureFiles=8),
    sixthSourceCounted=True, sourceFileAndDirectoryLinksRejected=True, sentinelPreserved=True,
    temporaryDirectoryRemoved=not work.exists())
(root/'docs/native-fixture-budget-check.json').write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps(report))
