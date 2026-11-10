"""Default-budget native visual-group pagination; 42 distinct tiny source files."""
from pathlib import Path
import hashlib
import importlib.util
import os
import struct
import sys
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-similar-pages-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-similar-pages-latest.json'
reuse.SCOPE = '42 distinct 9x8 grayscale PNGs across two sources, 21 paired dHash patterns; native exact-negative/visual20-to-21 pagination/type switching/last-group decision; no compilation or human calibration'
reuse.RUN_PREFIX = 'travel-gallery-native-similar-pages-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.PLANNED_FILES = 750
reuse.SOURCE_FILES = 42
reuse.b.STATE['imageFixtureUsed'] = False


def write_sources(source):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload))+kind+payload+struct.pack('>I', zlib.crc32(kind+payload)&0xffffffff)
    for family in range(21):
        pattern = int.from_bytes(hashlib.sha256(f'native-similar-pages-{family}'.encode()).digest()[:8], 'big')
        for offset, label in enumerate(('A', 'B')):
            rows = bytearray()
            for y in range(8):
                value = 100+offset
                rows.extend((0, value))
                for x in range(8):
                    bit = (pattern >> (63-y*8-x)) & 1
                    value += -10 if bit else 10
                    rows.append(value)
            data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 9, 8, 8, 0, 0, 0, 0))
            data += chunk(b'IDAT', zlib.compress(bytes(rows), level=0))+chunk(b'IEND', b'')
            folder = source/f'来源 {label} 📷'
            folder.mkdir(exist_ok=True)
            path = folder/f'{label}-pattern-{family:02}.png'
            path.write_bytes(data)
            os.utime(path, ns=(1711267200000000000, 1711267200000000000))


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
