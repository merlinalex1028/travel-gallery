"""Native restore rejection with a real GUI-paused scan; 24 compressed RGB PNGs, no builds."""
from pathlib import Path
import importlib.util
import struct
import sys
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-active-restore-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-active-restore-latest.json'
reuse.SCOPE = 'Native restore rejection while an actual GUI-started scan is paused; 24 compressed 8192x8192 RGB PNGs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-active-restore-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.SOURCE_FILES = 24
reuse.b.STATE['imageFixtureUsed'] = False


def write_sources(source):
    def chunk(kind, payload):
        return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload)&0xffffffff)
    for index in range(reuse.SOURCE_FILES):
        compressor = zlib.compressobj()
        row = b'\0'+bytes(((index*40)%256,80,120))*8192
        payload = bytearray()
        for _ in range(8192):
            payload.extend(compressor.compress(row))
        payload.extend(compressor.flush())
        data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',8192,8192,8,2,0,0,0))+chunk(b'IDAT',payload)+chunk(b'IEND',b'')
        (source/f'owned-{index}.png').write_bytes(data)
    assert sum(path.stat().st_size for path in source.iterdir()) < 10*1024**2


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
