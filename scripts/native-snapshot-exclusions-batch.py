"""Native frozen snapshot modifier-click exclusions; five tiny PNGs, no compilation."""
from pathlib import Path
import importlib.util
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-snapshot-exclusions-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-snapshot-exclusions-latest.json'
reuse.SCOPE = 'Native snapshot modifier-click exclusion, changed-query scope and explicit replacement; five tiny PNGs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-snapshot-exclusions-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.b.STATE['imageFixtureUsed'] = False

if __name__ == '__main__':
    raise SystemExit(reuse.run())
