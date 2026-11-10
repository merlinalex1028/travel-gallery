"""A stale image, hidden frame, or missing target must never qualify navigation."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('navigation_analysis', Path(__file__).resolve().parents[1] / 'analyze-native-navigation.py')
analysis = importlib.util.module_from_spec(spec)
spec.loader.exec_module(analysis)


class NavigationEvidence(unittest.TestCase):
    def setUp(self):
        self.manifest = [dict(id='a', relativePath='day/a.jpg'), dict(id='b', relativePath='day/b.jpg')]
        self.events = [dict(type='navigationProbeInstalled', previewHandleMarksSupported=True),
                       dict(type='screenPreviewResolved', assetId='b', ready=True, resourceUrl='media://localhost/b'),
                       dict(type='navigationDelivered', id=1, key='arrowright', deliveredAt=0, repeat=False,
                            focused=True, visibility='visible'),
                       dict(type='navigationHandled', id=1, accepted=True)]
        for at in [10, 27]:
            self.events.append(dict(type='viewerFrame', at=at, focused=True, visibility='visible', filename='b.jpg',
                                    image=dict(src='media://localhost/b', alt='b.jpg', complete=True, width=100, height=100)))

    def result(self, events=None):
        return analysis.analyze_events(events or self.events, self.manifest, ['a.jpg', 'b.jpg'])

    def test_actual_handle_matches_expected_decoded_asset(self):
        self.assertTrue(self.result()['completeNativeDomObservation'])
        self.assertEqual(self.result()['decodedTargetSecondRaf']['maxMs'], 27)

    def test_old_source_with_new_alt_is_a_mismatch(self):
        self.events.insert(1, dict(type='screenPreviewResolved', assetId='a', ready=True, resourceUrl='media://localhost/a'))
        self.events[-1]['image']['src'] = 'media://localhost/a'
        result = self.result()
        self.assertFalse(result['completeNativeDomObservation'])
        self.assertEqual(len(result['mismatchedImageFrames']), 1)

    def test_hidden_and_unknown_frames_do_not_pass(self):
        for change in ['hidden', 'unknown', 'hidden-input']:
            events = copy.deepcopy(self.events)
            if change == 'hidden':
                events[-1]['visibility'] = 'hidden'
            elif change == 'unknown':
                events[-1]['image']['src'] = 'media://localhost/not-observed'
            else:
                events[2]['visibility'] = 'hidden'
            self.assertFalse(self.result(events)['completeNativeDomObservation'])

    def test_only_one_decoded_frame_is_incomplete(self):
        self.events.pop()
        self.assertFalse(self.result()['allTargetsObservedDecoded'])

    def test_small_preview_identity_passes_but_is_not_counted_as_screen_quality(self):
        self.events[1].update(type='previewResolved', variant='small')
        result = self.result()
        self.assertTrue(result['completeNativeDomObservation'])
        self.assertEqual(result['distinctScreenDecodedPhotos'], 0)
        self.assertEqual(result['decodedFrameVariants']['small'], 2)
        self.assertEqual(result['resolvedScreenHandles'], 0)
        self.assertEqual(result['targets'][0]['previewVariantAtSecondRaf'], 'small')

    def test_stale_thumbnail_cannot_pass_under_a_different_photo_title(self):
        self.events[1].update(type='previewResolved', variant='small', assetId='a')
        result = self.result()
        self.assertFalse(result['completeNativeDomObservation'])
        self.assertEqual(len(result['mismatchedImageFrames']), 2)

    def test_response_identity_and_quality_must_match_the_actual_request(self):
        for fields in [dict(requestedAssetId='a'), dict(requestedVariant='small')]:
            events = copy.deepcopy(self.events)
            events[1].update(fields)
            with self.assertRaises(AssertionError):
                self.result(events)

    def test_early_frames_do_not_cover_a_later_unsampled_input(self):
        self.events.extend([
            dict(type='navigationDelivered', id=2, key='arrowleft', deliveredAt=1000, repeat=False,
                 focused=True, visibility='hidden'),
            dict(type='navigationHandled', id=2, accepted=True),
        ])
        result = self.result()
        self.assertFalse(result['coverageHasNoGapOver100ms'])
        self.assertFalse(result['completeNativeDomObservation'])


if __name__ == '__main__':
    unittest.main()
