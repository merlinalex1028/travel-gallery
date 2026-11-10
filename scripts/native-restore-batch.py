"""Reuse the accepted App and default-budget lifecycle for native restore boundaries."""
from pathlib import Path
import importlib.util
import sys

sys.dont_write_bytecode = True
root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('reuse', root/'scripts/native-disk-full-batch.py')
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = root/'docs/native-restore-default-budget.json'
reuse.b.RECEIPT = root/'docs/native-restore-latest.json'
reuse.SCOPE = 'Native invalid/changed backup rejection and genuine format-2 migration; five tiny PNGs; no image or compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-restore-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.b.STATE['imageFixtureUsed'] = False
raise SystemExit(reuse.run())
