"""The window verifier must distinguish a numbered image from stale pixels."""
import importlib.util
from pathlib import Path
import struct
import unittest


def load(name):
    scripts = Path(__file__).resolve().parents[1]
    spec = importlib.util.spec_from_file_location(name, scripts / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pixels = load('native_navigation_pixels')
focus = load('native-navigation-focus-batch')


class WindowPixels(unittest.TestCase):
    def test_full_window_accepts_current_pixels_but_rejects_stale_and_clipped_images(self):
        cols, rows = 804, 600
        photo_width, photo_height = 640, 400
        left, top = 82, 102
        image = bytearray(cols * rows * 3)
        colour = focus.colour(25)
        row_bytes = bytes(reversed(colour)) * photo_width
        for y in range(top, top + photo_height):
            start = (y * cols + left) * 3
            image[start:start + len(row_bytes)] = row_bytes

        def point(x, y):
            return ((top + y * photo_height // focus.HEIGHT) * cols
                    + left + x * photo_width // focus.WIDTH) * 3

        def render(number):
            frame = image[:]
            for digit, value in enumerate(f'{number:03}'):
                for cell, enabled in enumerate(focus.DIGITS[int(value)]):
                    if enabled == '1':
                        start = point(415 + digit * 300 + cell % 3 * 70, 330 + cell // 3 * 70)
                        frame[start:start + 3] = b'\xff\xff\xff'
            return frame

        def verify(frame):
            header = struct.pack('<2sIII', b'BM', len(frame) + 54, 0, 54)
            info = struct.pack('<IiiHHIIIIII', 40, cols, -rows, 1, 24, 0, len(frame), 0, 0, 0, 0)
            return pixels.check_window_pixels(header + info + frame, focus.WIDTH, focus.HEIGHT,
                                              colour, [focus.DIGITS[n] for n in (0, 2, 5)])

        current = render(25)
        self.assertEqual(verify(current), {'windowPixels': [cols, rows], 'photoPixels': [640, 400], 'digitCells': 45})
        with self.assertRaisesRegex(ValueError, 'Window pixels differ'):
            verify(render(24))
        clipped = current[:]
        clipped[:cols * (top + 15) * 3] = bytes(cols * (top + 15) * 3)
        with self.assertRaisesRegex(ValueError, 'aspect ratio'):
            verify(clipped)
        with self.assertRaisesRegex(ValueError, 'truncated'):
            verify(current[:-3])
