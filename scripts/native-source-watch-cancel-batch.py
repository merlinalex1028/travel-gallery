"""Native source disable/cancel/re-enable lifecycle with real work; default budget, no builds."""
from pathlib import Path
import importlib.util
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('active_fixture', Path(__file__).with_name('native-active-restore-batch.py'))
active_fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(active_fixture)
reuse = active_fixture.reuse
reuse.b.APPROVAL = reuse.ROOT/'docs/native-source-watch-cancel-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-source-watch-cancel-latest.json'
reuse.SCOPE = 'Native real paused scan, disable source and observe zero watched roots, cancel, preserve manual records, re-enable automatic catch-up; 24 compressed PNGs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-source-watch-cancel-'

if __name__ == '__main__':
    raise SystemExit(reuse.run())
