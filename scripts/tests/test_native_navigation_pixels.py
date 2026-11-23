"""The window verifier must distinguish a numbered image from stale pixels."""
import importlib.util
from pathlib import Path
import struct
import unittest
from unittest.mock import patch


def load(name):
    scripts = Path(__file__).resolve().parents[1]
    spec = importlib.util.spec_from_file_location(name, scripts / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pixels = load('native_navigation_pixels')
focus = load('native-navigation-focus-batch')
joint = load('native-navigation-joint-batch')


class ForegroundPreflight(unittest.TestCase):
    def setUp(self):
        self.start = dict(atUtc='2026-11-23T07:00:00+00:00', nativeElapsedMs=1000, runId='observer')
        self.clock = dict(type='clock', runId='observer', nativeRespondedMs=3000,
                          visibility='visible', focused=True)
        self.now = joint.datetime.fromisoformat(self.start['atUtc']).timestamp() * 1000 + 3000

    def test_only_fresh_visible_clock_of_this_run_passes(self):
        self.assertTrue(joint.visible_clock(self.start, [self.clock], self.now))
        self.assertFalse(joint.visible_clock(self.start, [self.clock], self.now + 5000))
        self.assertFalse(joint.visible_clock(self.start, [self.clock], self.now - 2000))
        for fields in [dict(runId='another'), dict(visibility='hidden'), dict(focused=False)]:
            with self.subTest(fields=fields):
                latest = dict(self.clock, **fields)
                self.assertFalse(joint.visible_clock(self.start, [self.clock, latest], self.now))

    def test_foreground_must_hold_across_same_pid_webview_check(self):
        with (patch.object(joint, 'require_unlocked_console'),
              patch.dict(joint.b.STATE, verifiedPid=123),
              patch.object(joint.focus.picker, 'verified_process', return_value=123),
              patch.object(joint, 'read_probe', return_value=(b'', self.start, [self.clock])),
              patch.object(joint.time, 'time_ns', return_value=int(self.now * 1_000_000))):
            qa, other = 'ASN:qa', 'ASN:other'
            with patch.object(joint, 'front_asn', side_effect=[qa, qa, other]):
                with self.assertRaisesRegex(ValueError, 'lost foreground'):
                    joint.confirm_foreground()
            with patch.object(joint, 'front_asn', side_effect=[qa, qa, qa]):
                self.assertEqual(joint.confirm_foreground(), 123)


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
