"""Native indexed-time/rating/wall-date sorting across 120-item pagination."""
from pathlib import Path
import datetime
import importlib.util
import os
import struct
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
reuse.b.APPROVAL = reuse.ROOT/'docs/native-sorting-default-budget.json'
reuse.b.RECEIPT = reuse.ROOT/'docs/native-sorting-latest.json'
reuse.SCOPE = 'Native rating, indexed-time and mixed known/unknown wall-date sorting; 125 tiny TIFFs; no compilation'
reuse.RUN_PREFIX = 'travel-gallery-native-sorting-'
reuse.PLANNED_BYTES = 512*1024**2
reuse.PLANNED_FILES = 750
reuse.SOURCE_FILES = 125
reuse.b.STATE['imageFixtureUsed'] = False


def tiny_tiff(index, capture=None, offset=None):
    count = 12 if capture else 10
    data = bytearray(b'II*\x00'+struct.pack('<I', 8)+bytes(2+12*count+4))

    def append(payload):
        if len(data) % 2:
            data.append(0)
        position = len(data)
        data.extend(payload)
        return position

    bits = append(struct.pack('<HHH', 8, 8, 8))
    exif = []
    if capture:
        date = append(capture.encode()+b'\x00')
        exif.append((0x9003, 2, 20, date))
        if offset:
            zone = (offset+'\x00').encode()
            exif.append((0x9011, 2, len(zone), append(zone)))
        exif_ifd = append(struct.pack('<H', len(exif))+
            b''.join(struct.pack('<HHII', *entry) for entry in exif)+struct.pack('<I', 0))
    pixel = append(bytes((index % 256, (index*3) % 256, (index*7) % 256)))
    entries = [(256, 4, 1, 1), (257, 4, 1, 1), (258, 3, 3, bits),
        (259, 3, 1, 1), (262, 3, 1, 2), (273, 4, 1, pixel),
        (277, 3, 1, 3), (278, 4, 1, 1), (279, 4, 1, 3), (284, 3, 1, 1)]
    if capture:
        entries += [(306, 2, 20, date), (34665, 4, 1, exif_ifd)]
    entries.sort()
    data[8:8+2+12*count+4] = struct.pack('<H', count)+b''.join(
        struct.pack('<HHII', *entry) for entry in entries)+struct.pack('<I', 0)
    return bytes(data)


def write_sources(source):
    for index in range(125):
        capture, offset = ('2024:03:24 08:00:00', None) if index < 121 else (
            ('2024:03:23 23:00:00', '+08:00') if index == 121 else
            ('2024:03:24 10:00:00', '+08:00') if index == 122 else (None, None))
        path = source/f'sort-{index:03}.tiff'
        path.write_bytes(tiny_tiff(index, capture, offset))
        stamp = datetime.datetime(2024, 3, 24 if index != 124 else 22, 8, tzinfo=datetime.timezone.utc).timestamp()
        os.utime(path, ns=(int(stamp*10**9), int(stamp*10**9)))


reuse.write_sources = write_sources

if __name__ == '__main__':
    raise SystemExit(reuse.run())
