"""Create one genuine schema-2 backup from the current isolated five-item QA catalog."""
from pathlib import Path
import hashlib
import json
import sqlite3

root = Path(__file__).resolve().parent.parent
state = json.loads((root/'docs/native-restore-legacy-latest.json').read_bytes())
assert state['status'] == 'ready'
work = Path(state['temporaryRoot'])
assert work.is_dir() and work.name.startswith('travel-gallery-native-restore-legacy-')
source = sqlite3.connect(f"file:{state['expectedDatabase']}?mode=ro", uri=True)
source.execute('BEGIN')
assert source.execute('select count(*) from media_assets').fetchone()[0] == 5
assert source.execute("select count(*) from asset_reviews where pick_state='keep'").fetchone()[0] == 5
assert source.execute("select count(*) from jobs where status in ('queued','running','paused')").fetchone()[0] == 0
directory = work/'fixture/backups'
directory.mkdir()
path = directory/'legacy-v2.sqlite'
migrations = [root/'src-tauri/migrations'/name for name in ('001_baseline.sql', '002_interactions.sql')]
assert all(p.is_file() for p in migrations)
target = sqlite3.connect(path)
try:
    for migration in migrations:
        target.executescript(migration.read_text())
    target.execute('PRAGMA foreign_keys=OFF')
    target.execute('PRAGMA user_version=2')
    tables = [r[0] for r in target.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%'")]
    for table in tables:
        columns = [r[1] for r in target.execute(f'PRAGMA table_info("{table}")')]
        source_columns = {r[1] for r in source.execute(f'PRAGMA table_info("{table}")')}
        shared = [name for name in columns if name in source_columns]
        if not shared:
            continue
        names = ','.join(f'"{name}"' for name in shared)
        rows = source.execute(f'SELECT {names} FROM "{table}"').fetchall()
        target.executemany(f'INSERT OR REPLACE INTO "{table}" ({names}) VALUES ({",".join("?" for _ in shared)})', rows)
    target.commit()
    assert target.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
    assert not target.execute('PRAGMA foreign_key_check').fetchall()
    assert target.execute('PRAGMA user_version').fetchone()[0] == 2
    manual = ('asset_reviews','tags','asset_tags','linked_media','collections','collection_assets','saved_filters','app_settings')
    expected = {table:dict(columns=[r[1] for r in target.execute(f'PRAGMA table_info("{table}")')],
        count=target.execute(f'SELECT COUNT(*) FROM "{table}"').fetchone()[0],
        sha256=hashlib.sha256('\n'.join(sorted(json.dumps(tuple(r),ensure_ascii=False,sort_keys=True)
            for r in target.execute(f'SELECT * FROM "{table}"'))).encode()).hexdigest()) for table in manual}
    ids = [r[0] for r in target.execute('select id from media_assets order by id')]
    assert ids == [r[0] for r in source.execute('select id from media_assets order by id')]
except BaseException:
    target.close()
    path.unlink(missing_ok=True)
    raise
finally:
    target.close()
    source.close()
record = dict(status='legacy-fixture-prepared', release=state['acceptedRelease'], sourceBefore=state['sourceBefore'],
    fixture=dict(path=str(path),version=2,bytes=path.stat().st_size,sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                 integrity='ok',foreignKeyErrors=0,migrationSources={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in migrations}),
    expectedMediaIds=ids,expectedManualTables=expected)
(root/'docs/native-restore-legacy-observations.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(dict(fixture=str(path),version=2,media=len(ids),integrity='ok',foreignKeyErrors=0,bytes=path.stat().st_size)))
