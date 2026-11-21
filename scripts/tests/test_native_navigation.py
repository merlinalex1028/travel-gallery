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
        self.assertTrue(self.result()['allTargetsObservedScreenDecoded'])

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
        self.assertFalse(result['allTargetsObservedScreenDecoded'])

    def test_later_visit_cannot_supply_frames_for_an_earlier_input(self):
        del self.events[-2:]
        self.events.append(dict(type='screenPreviewResolved', assetId='a', ready=True, resourceUrl='media://localhost/a'))
        for ident, key, at, name in [(2, 'arrowleft', 50, 'a'), (3, 'arrowright', 100, 'b')]:
            self.events.extend([
                dict(type='navigationDelivered', id=ident, key=key, deliveredAt=at, repeat=False,
                     focused=True, visibility='visible'),
                dict(type='navigationHandled', id=ident, accepted=True),
                *(dict(type='viewerFrame', navigationId=ident, at=at + delay, focused=True, visibility='visible',
                       filename=f'{name}.jpg', image=dict(src=f'media://localhost/{name}', alt=f'{name}.jpg',
                                                           complete=True, width=100, height=100)) for delay in (10, 27)),
            ])
        result = self.result()
        self.assertEqual(result['missingDecodedTargets'], [dict(id=1, filename='b.jpg')])
        self.assertEqual(result['screenDecodedTargetIds'], [2, 3])
        self.assertFalse(result['completeNativeDomObservation'])

    def test_frames_with_a_stale_navigation_id_do_not_count(self):
        self.events[-1]['navigationId'] = 0
        self.events[-2]['navigationId'] = 0
        self.assertFalse(self.result()['allTargetsObservedDecoded'])
        self.assertFalse(self.result()['allTargetsObservedScreenDecoded'])

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


class FocusVariantEvidence(unittest.TestCase):
    def setUp(self):
        self.names = ['a.jpg', 'b.jpg', 'c.jpg']
        self.manifest = [dict(id=name, relativePath=name) for name in self.names]
        self.events = [dict(type='navigationProbeInstalled', previewHandleMarksSupported=True)]
        for ident, start, name in [(1, 0, 'b.jpg'), (2, 450, 'c.jpg')]:
            self.events.extend([
                dict(type='previewResolved', variant='screen', assetId=name, ready=True,
                     resourceUrl=f'media://localhost/{name}'),
                dict(type='navigationDelivered', id=ident, key='arrowright', deliveredAt=start,
                     repeat=False, visibility='visible', focused=True),
                dict(type='navigationHandled', id=ident, accepted=True),
                dict(type='blur', at=start + 35, focused=False),
                dict(type='focus', at=start + 360, focused=True),
                *(dict(type='viewerFrame', navigationId=ident, at=start + at,
                       filename=name, focused=at != 120, visibility='hidden' if at == 120 else 'visible',
                       image=dict(src=f'media://localhost/{name}', alt=name, complete=True, width=100, height=100))
                  for at in (10, 27, 120, 376, 392, 440)),
            ])

    def result(self):
        return analysis.analyze_events(self.events, self.manifest, self.names,
                                       required_focus_returns=2, expected_focus_photos=2)

    def test_observed_returns_allow_only_the_bracketed_background_gaps(self):
        result = self.result()
        focus = result['focusVariant']
        self.assertFalse(result['completeNativeDomObservation'])
        self.assertTrue(focus['completeDomObservation'])
        self.assertEqual([trip['secondRafAt'] for trip in focus['roundTrips']], [392, 842])
        self.events = [e for e in self.events if e['type'] != 'blur' or e['at'] != 485]
        self.assertFalse(self.result()['focusVariant']['completeDomObservation'])

    def test_unobserved_foreground_gap_invalidates_the_focus_variant(self):
        for event in self.events[8:]:
            if event.get('at', 0) >= 450:
                event['at'] += 150
            if event.get('deliveredAt', 0) >= 450:
                event['deliveredAt'] += 150
        focus = self.result()['focusVariant']
        self.assertEqual(focus['distinctPhotosWithObservedReturn'], 2)
        self.assertFalse(focus['foregroundCoverageComplete'])
        self.assertFalse(focus['completeDomObservation'])

    def test_previous_photo_on_return_is_not_a_valid_recovery(self):
        frame = next(e for e in self.events if e['type'] == 'viewerFrame' and e['at'] == 826)
        frame.update(filename='b.jpg')
        frame['image'].update(src='media://localhost/b.jpg', alt='b.jpg')
        self.assertFalse(self.result()['focusVariant']['completeDomObservation'])

    def test_extra_stale_return_invalidates_an_otherwise_complete_run(self):
        stale = copy.deepcopy(self.events[-1])
        stale.update(at=935, filename='b.jpg')
        stale['image'].update(src='media://localhost/b.jpg', alt='b.jpg')
        self.events.extend([dict(type='blur', at=900, focused=False),
                            dict(type='focus', at=930, focused=True), stale])
        focus = self.result()['focusVariant']
        self.assertEqual(focus['distinctPhotosWithObservedReturn'], 2)
        self.assertEqual(focus['staleReturnFrames'], [dict(id=2, expected='c.jpg', at=935, filename='b.jpg')])
        self.assertFalse(focus['completeDomObservation'])

    def test_small_only_target_or_short_run_cannot_qualify(self):
        self.assertFalse(analysis.analyze_events(self.events, self.manifest, self.names)
                         ['focusVariant']['completeDomObservation'])
        preview = next(e for e in self.events if e.get('assetId') == 'c.jpg')
        preview['variant'] = 'small'
        self.assertFalse(self.result()['focusVariant']['completeDomObservation'])


class OSFocusEvidence(unittest.TestCase):
    def setUp(self):
        self.native = dict(pid=4321, identifier='com.travelgallery.benchmark', runId='owned-test',
                           atUtc='2023-11-14T22:13:20+00:00', nativeElapsedMs=50)
        epoch = 1_700_000_000_000
        self.events = [dict(type='probeInstalled', timeOrigin=epoch + 25),
                       dict(type='clock', runId='owned-test', sent=50, received=90, nativeReceivedMs=140)]
        self.dom = dict(completeDomObservation=True, roundTrips=[
            dict(id=1, filename='a.jpg', blurAt=100, focusAt=500),
            dict(id=2, filename='b.jpg', blurAt=800, focusAt=1200),
        ])
        self.os = dict(pid=4321, identifier='com.travelgallery.benchmark', samples=20, errors=[],
                       unixOffsetMs=epoch - 10000, firstSampleStartedMonotonic=10,
                       lastSampleFinishedMonotonic=11.7, maximumOffsetDriftMs=1, maximumSampleGapMs=100,
                       transitions=[dict(role='qa', atMonotonic=10.05),
                                    *[dict(role=role, earliestMonotonic=at - .08, atMonotonic=at)
                                      for role, at in [('other', 10.16), ('qa', 10.56),
                                                       ('other', 10.86), ('qa', 11.26)]]])

    def result(self):
        return analysis.corroborate_os_focus(self.dom, self.os, self.native, self.events)

    def test_matches_same_process_clock_and_each_ordered_return(self):
        self.assertEqual(self.result(), dict(complete=True, reason=None, matches=[
            dict(id=1, filename='a.jpg'), dict(id=2, filename='b.jpg')]))

    def test_same_count_outside_the_dom_intervals_does_not_count(self):
        for item in self.os['transitions'][1:]:
            item['atMonotonic'] += 4
            item['earliestMonotonic'] += 4
        self.os['lastSampleFinishedMonotonic'] += 4
        self.assertFalse(self.result()['complete'])

    def test_a_native_return_cannot_cover_two_photos(self):
        self.dom['roundTrips'][1].update(blurAt=105, focusAt=505)
        result = self.result()
        self.assertFalse(result['complete'])
        self.assertEqual(result['matches'], [dict(id=1, filename='a.jpg')])

    def test_rejects_foreign_pid_clock_drift_and_sampling_gap(self):
        for override in [dict(pid=1234), dict(maximumOffsetDriftMs=51), dict(maximumSampleGapMs=351)]:
            with self.subTest(override=override):
                self.assertFalse(analysis.corroborate_os_focus(
                    self.dom, dict(self.os, **override), self.native, self.events)['complete'])

    def test_rejects_an_uncorroborated_webview_clock_or_frame_trace(self):
        self.events[1]['nativeReceivedMs'] += 1000
        self.assertFalse(self.result()['complete'])
        self.events[1]['nativeReceivedMs'] -= 1000
        self.dom['completeDomObservation'] = False
        self.assertFalse(self.result()['complete'])


if __name__ == '__main__':
    unittest.main()
