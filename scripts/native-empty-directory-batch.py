"""Native empty-directory picker comparison using the accepted App; no compilation."""
from pathlib import Path
import importlib.util
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-empty-directory-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-empty-directory-latest.json'
reuse.SCOPE = 'Native empty and nonempty export-directory picker comparison; five tiny PNGs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-empty-directory-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.b.STATE['imageFixtureUsed'] = False
original_sources = reuse.write_sources


def write_sources(source):
    original_sources(source)
    for name in ('empty-target', 'second-empty-target', 'existing-target'):
        (source.parent/name).mkdir()
    (source.parent/'existing-target/keep.txt').write_bytes(b'owned target sentinel\n')


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
