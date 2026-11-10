"""Reject partial or idle traces as continuous native scrolling evidence."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


module = importlib.util.spec_from_file_location(
    'native_analysis', Path(__file__).resolve().parents[1] / 'analyze-native-performance.py')
analysis = importlib.util.module_from_spec(module)
module.loader.exec_module(analysis)


def segment(end_ms, frame_until, scroll_times):
    rows = [dict(type='nativeStart', runId='owned')]

    def add(event):
        rows.append(dict(type='frontendBatchEvent', runId='owned', nativeReceivedMs=event['at'],
                         event=dict(focused=True, visibility='visible', **event)))

    add(dict(type='probeInstalled', at=0, timeOrigin=0, longTaskSupported=False))
    add(dict(type='segmentStart', at=0, id=1, start=0))
    for at in range(0, frame_until + 1, 20):
        add(dict(type='frame', at=at, timestamp=at, intervalMs=20 if at else None, segmentId=1))
    for at in scroll_times:
        add(dict(type='viewportScroll', at=at, scrollTop=at))
    add(dict(type='domSample', at=0, allElements=800, cards=27, indexedCount='50,000'))
    add(dict(type='segmentEnd', at=end_ms, id=1))
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'raw.jsonl'
        path.write_text('\n'.join(map(json.dumps, rows)) + '\n')
        return analysis.analyze(path)['segments'][0]


class ContinuousEvidence(unittest.TestCase):
    def test_complete_observation_and_input_cadence(self):
        result = segment(60000, 59980, range(0, 60000, 500))
        self.assertTrue(result['complete60SecondFrameWindow'])
        self.assertTrue(result['continuousScrollInputObserved'])
        self.assertIsNone(result['longTaskCount'])

    def test_idle_rendering_does_not_prove_continuous_input(self):
        result = segment(60000, 59980, [0, 30000])
        self.assertTrue(result['complete60SecondFrameWindow'])
        self.assertFalse(result['continuousScrollInputObserved'])

    def test_late_end_event_does_not_hide_missing_frames(self):
        result = segment(137000, 49000, range(0, 49000, 500))
        self.assertFalse(result['complete60SecondFrameWindow'])
        self.assertFalse(result['continuousScrollInputObserved'])


if __name__ == '__main__':
    unittest.main()
