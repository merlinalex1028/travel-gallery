"""Default-budget native pagination and explicit scope of a 125-member exact group."""
from pathlib import Path
import importlib.util
import struct
import sys
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-large-group-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-large-group-latest.json'
reuse.SCOPE = '125 identical tiny PNGs across two real imported sources; member pagination and displayed-member explicit actions; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-large-group-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.PLANNED_FILES = 750
reuse.SOURCE_FILES = 125
reuse.b.STATE['imageFixtureUsed'] = False


def write_sources(source):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload))+kind+payload+struct.pack('>I', zlib.crc32(kind+payload)&0xffffffff)
    data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0))
    data += chunk(b'IDAT', zlib.compress(bytes([0, 40, 80, 120, 255])))+chunk(b'IEND', b'')
    for index in range(125):
        label = 'A' if index < 63 else 'B'
        folder = source/f'来源 {label} 📷'
        folder.mkdir(exist_ok=True)
        (folder/f'{label}-{index:03}.png').write_bytes(data)


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
