"""Default-budget native outer duplicate-group pagination, using the accepted App."""
from pathlib import Path
import importlib.util
import struct
import sys
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-group-pages-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-group-pages-latest.json'
reuse.SCOPE = '21 exact SHA-256 families, two tiny PNGs each across two imported sources; outer 20-to-21 group pagination and last-group actions; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-group-pages-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.PLANNED_FILES = 750
reuse.SOURCE_FILES = 42
reuse.b.STATE['imageFixtureUsed'] = False


def write_sources(source):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload))+kind+payload+struct.pack('>I', zlib.crc32(kind+payload)&0xffffffff)
    for family in range(21):
        data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0))
        data += chunk(b'IDAT', zlib.compress(bytes([0, family*10, 80, 120, 255])))+chunk(b'IEND', b'')
        for label in ('A', 'B'):
            folder = source/f'来源 {label} 📷'
            folder.mkdir(exist_ok=True)
            (folder/f'{label}-family-{family:02}.png').write_bytes(data)


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
