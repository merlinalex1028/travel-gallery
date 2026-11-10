"""Native cross-source group review, undo and exclusion persistence; five tiny PNGs."""
from pathlib import Path
import importlib.util
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('cross_fixture', Path(__file__).with_name('native-cross-source-duplicates-batch.py'))
cross_fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cross_fixture)
reuse = cross_fixture.reuse
reuse.b.APPROVAL = reuse.ROOT/'docs/native-cross-source-actions-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-cross-source-actions-latest.json'
reuse.SCOPE = 'Native cross-source group explicit reject confirmation/cancel, undo and exclusion/recheck decisions; five tiny PNGs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-cross-source-actions-'

if __name__ == '__main__':
    raise SystemExit(reuse.run())
