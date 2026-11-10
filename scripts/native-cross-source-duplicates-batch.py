"""Native cross-source exact groups, same-name negatives and offline representative; five tiny PNGs."""
from pathlib import Path
import importlib.util
import struct
import sys
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-cross-source-duplicates-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-cross-source-duplicates-latest.json'
reuse.SCOPE = 'Native cross-source byte duplicates and same-name different-content negatives, representative source offline/reconnect; five tiny PNGs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-cross-source-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.b.STATE['imageFixtureUsed'] = False


def write_sources(source):
    def chunk(kind, payload):
        return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload)&0xffffffff)
    for relative, color in [('来源 A/travel.png',(40,80,120,255)),
                            ('来源 B/travel-copy.png',(40,80,120,255)),
                            ('来源 A/same-name.png',(10,20,30,255)),
                            ('来源 B/same-name.png',(20,40,60,255)),
                            ('来源 A/unique.png',(80,90,100,255))]:
        path=source/relative
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+
                         chunk(b'IDAT',zlib.compress(bytes([0,*color])))+chunk(b'IEND',b''))


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
