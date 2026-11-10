"""Verify a native rejection left the isolated QA catalog/source unchanged; no app actions."""
from pathlib import Path
import argparse
import hashlib
import json
import sqlite3

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('case')
parser.add_argument('message')
args = parser.parse_args()
state = json.loads((root/'docs/native-restore-latest.json').read_bytes())
assert state['status'] == 'ready'
work = Path(state['temporaryRoot'])
path = root/'docs/native-restore-observations.json'
record = json.loads(path.read_bytes())
before = record['beforeNegativeCases']
db = Path(state['expectedDatabase'])
with sqlite3.connect(f'file:{db}?mode=ro', uri=True) as conn:
    conn.execute('BEGIN')
    current = dict(version=conn.execute('PRAGMA user_version').fetchone()[0],
        media=conn.execute('SELECT COUNT(*) FROM media_assets').fetchone()[0],
        tables={table: hashlib.sha256('\n'.join(sorted(json.dumps(tuple(row), ensure_ascii=False, sort_keys=True)
            for row in conn.execute(f'SELECT * FROM {table}'))).encode()).hexdigest() for table in before['tables']},
        reviewRows=[list(row) for row in conn.execute('SELECT * FROM asset_reviews ORDER BY asset_id')],
        automaticBackups=[p.name for p in db.parent.glob('before-restore-*.sqlite')])
assert current == before, 'Rejected backup changed the live QA catalog or created an automatic backup'
sources = []
for p in sorted((work/'fixture/source').glob('*')):
    stat = p.stat()
    sources.append(dict(name=p.name, bytes=stat.st_size, sha256=hashlib.sha256(p.read_bytes()).hexdigest(),
        mtimeNs=stat.st_mtime_ns, inode=stat.st_ino, links=stat.st_nlink))
assert sources == record['sourceBefore'], 'Original test source changed'
record.setdefault('negativeCases', []).append(dict(case=args.case, visibleMessage=args.message,
    catalogVersion=9, media=5, nineTablesPreserved=True, sourceFivePreserved=True, automaticBackupsAdded=0))
path.write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n')
print(json.dumps(record['negativeCases'][-1], ensure_ascii=False))
