"""Check the picker controller against an owned SQLite catalog, without opening UI."""
from contextlib import ExitStack, closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('preflight', ROOT / 'scripts/native-restore-picker-preflight.py')
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class RestorePickerPreflight(unittest.TestCase):
    def exercise(self, *, foreground=True):
        with (tempfile.TemporaryDirectory(prefix='travel-gallery-picker-controller-') as temporary,
              ExitStack() as stack):
            raw = Path(temporary)
            work = raw.resolve()
            (work / 'control').mkdir()
            (work / 'backup').mkdir()
            (work / 'data').mkdir()
            database = work / 'data/catalog.sqlite'
            db = stack.enter_context(closing(sqlite3.connect(database)))
            db.executescript('''
                CREATE TABLE media_assets(id TEXT);
                CREATE TABLE source_roots(absolute_path TEXT);
                CREATE TABLE jobs(status TEXT);
            ''')
            state = dict(normalBefore={}, sourceBefore=[])
            completed = []

            def event(stage, **details):
                if stage == 'nativePickerPreflightCheckpoint':
                    completed.append(details['step'])
                if stage != 'awaitingNativeCheckpoint':
                    return
                step = details['step']
                request = dict(step=step)
                if step == 'imported':
                    with db:
                        db.execute('INSERT INTO source_roots VALUES(?)', (str(work / 'fixture/source'),))
                        db.executemany('INSERT INTO media_assets VALUES(?)', [(str(i),) for i in range(5)])
                if step == 'backed-up':
                    with closing(sqlite3.connect(work / 'backup/travel-gallery-owned.sqlite')) as backup:
                        db.backup(backup)
                if step == 'file-reviewing':
                    request['axFrame'] = f"{state['pickerPortableBackup']}\nbutton 备份当前库并恢复"
                (work / 'control/step.json').write_text(json.dumps(request))

            def output(args, **kwargs):
                if args[0] == 'ps':
                    return ''
                return 'ASN:qa:' if args[-1] != 'front' or foreground else 'ASN:other:'

            with (patch.multiple(preflight.b, WORK=raw, DATA=work / 'data',
                                 APP_PATH=work / 'qa.app', STATE=state, event=event),
                  patch.object(preflight.b, 'check', return_value={}),
                  patch.object(preflight.reuse, 'normal_state', return_value={}),
                  patch.object(preflight.reuse, 'source_state', return_value=[]),
                  patch.object(preflight.picker, 'verified_process', return_value=42),
                  patch.object(preflight.guard, 'require_unlocked_console'),
                  patch.object(preflight.subprocess, 'check_output', side_effect=output)):
                preflight.accept()
            self.assertEqual(completed, ['empty', 'imported', 'backed-up', 'file-reviewing'])
            self.assertTrue(state['pickerPreflightPassed'] and state['qaQuitNormally'])
            self.assertEqual(Path(state['pickerPortableBackup']).read_bytes()[18:20], b'\x01\x01')

    def test_native_canonical_paths_and_single_file_confirmation_complete_the_preflight(self):
        self.exercise()

    def test_other_front_application_blocks_before_import(self):
        with self.assertRaisesRegex(AssertionError, 'foreground before import'):
            self.exercise(foreground=False)


if __name__ == '__main__':
    unittest.main()
