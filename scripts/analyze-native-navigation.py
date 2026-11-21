"""Correlate native viewer RAF samples with actual preview responses and owned IDs."""
import argparse
from bisect import bisect_left
from datetime import datetime
import hashlib
import json
import math
from pathlib import Path


def decoded_pair(frames, matches):
    return next(((a, b) for a, b in zip(frames, frames[1:])
                 if b['at'] - a['at'] <= 100 and matches(a) and matches(b)), None)


def foreground_coverage(start, end, frames, round_trips):
    """Exclude only observed blur-to-focus intervals, not arbitrary frame gaps."""
    segments = list(zip([start, *(trip['focusAt'] for trip in round_trips)],
                        [*(trip['blurAt'] for trip in round_trips), end]))
    times = [f['at'] for f in frames if f['visibility'] == 'visible' and f['focused']]
    for left, right in segments:
        samples = [t for t in times if left <= t <= right]
        points = [left, *samples, right]
        if not samples or any(b - a > 100 for a, b in zip(points, points[1:])):
            return False
    return all((f['visibility'] == 'visible' and f['focused']) or
               any(trip['blurAt'] <= f['at'] < trip['focusAt'] for trip in round_trips)
               for f in frames)


def corroborate_os_focus(dom, observation, native_start, events):
    """Pair ordered DOM round trips with distinct transitions in the same OS process."""
    def incomplete(reason, matches=()):
        return dict(complete=False, reason=reason, matches=list(matches))

    process = (native_start['pid'], native_start['identifier'])
    if process != (observation.get('pid'), observation.get('identifier')):
        return incomplete('OS observation belongs to a different process')
    installed = [event for event in events if event['type'] == 'probeInstalled']
    if len(installed) != 1 or 'timeOrigin' not in installed[0]:
        return incomplete('The WebView clock origin is missing or ambiguous')
    if (observation.get('errors') or observation.get('samples', 0) < 2
            or observation.get('maximumOffsetDriftMs', math.inf) > 50
            or observation.get('maximumSampleGapMs', math.inf) > 350):
        return incomplete('OS focus sampling or clock coverage is incomplete')
    origin = installed[0]['timeOrigin']
    offset = observation['unixOffsetMs']
    start = observation['firstSampleStartedMonotonic'] * 1000 + offset
    end = observation['lastSampleFinishedMonotonic'] * 1000 + offset
    native_epoch = datetime.fromisoformat(native_start['atUtc']).timestamp() * 1000
    native_origin = native_epoch - native_start['nativeElapsedMs']
    clock_samples = [event for event in events if event['type'] == 'clock' and event['runId'] == native_start['runId']
                     and 0 <= event['received'] - event['sent'] <= 200]

    def clock_agrees(event):
        at = native_origin + event['nativeReceivedMs']
        return origin + event['sent'] - 100 <= at <= origin + event['received'] + 100

    if (not clock_samples or not all(map(clock_agrees, clock_samples))
            or start < native_epoch - 100 or start >= end):
        return incomplete('OS and WebView observations do not share a valid clock window')

    transitions = observation['transitions']
    pairs = [(away, back) for before, away, back in zip(transitions, transitions[1:], transitions[2:])
             if (before['role'], away['role'], back['role']) == ('qa', 'other', 'qa')]

    def compatible(dom_time, transition):
        earliest = transition['earliestMonotonic'] * 1000 + offset
        latest = transition['atMonotonic'] * 1000 + offset
        return start <= earliest <= latest <= end and latest - earliest <= 350 and (
            earliest - 100 <= dom_time <= latest + 100)

    matches, next_pair = [], 0
    for trip in dom['roundTrips']:
        blur, focus = origin + trip['blurAt'], origin + trip['focusAt']
        matching = [i for i, (away, back) in enumerate(pairs) if i >= next_pair
                    and start <= blur < focus <= end and compatible(blur, away) and compatible(focus, back)]
        if len(matching) != 1:
            return incomplete(f'No unique OS focus return for navigation {trip["id"]}', matches)
        next_pair = matching[0] + 1
        matches.append(dict(id=trip['id'], filename=trip['filename']))
    if not matches or not dom['completeDomObservation']:
        return incomplete('WebView focus and decoded frame evidence is incomplete', matches)
    return dict(complete=True, reason=None, matches=matches)


def analyze_events(events, manifest, order, first_position=1, start_id=1, end_id=1000000, *,
                   required_focus_returns=20, expected_focus_photos=100):
    assert 1 <= required_focus_returns <= expected_focus_photos
    names = {item['id']: Path(item['relativePath']).name for item in manifest}
    assert len(set(names.values())) == len(names)
    assert set(order) == set(names.values()) and len(order) == len(names)
    installed = [e for e in events if e['type'] == 'navigationProbeInstalled']
    assert len(installed) == 1
    inputs = [e for e in events if e['type'] == 'navigationDelivered' and start_id <= e['id'] <= end_id]
    deliveries = [e['deliveredAt'] for e in inputs]
    assert all(a < b for a, b in zip(deliveries, deliveries[1:])), 'Navigation times must be strictly ordered.'
    handled = [e for e in events if e['type'] == 'navigationHandled' and start_id <= e['id'] <= end_id]
    assert [e['id'] for e in inputs] == list(range(start_id, start_id + len(inputs)))
    assert len({e['id'] for e in handled}) == len(handled)
    accepted = {e['id']: e['accepted'] for e in handled}
    targets = []
    position = first_position - 1
    for event in inputs:
        assert event['key'] in ['arrowleft', 'arrowright']
        position += 1 if event['key'] == 'arrowright' else -1
        assert 0 <= position < len(order), 'Run must not navigate past the known fixture boundary.'
        targets.append((event, order[position]))
    handles, variants = {}, {}
    for event in events:
        if event['type'] in ['screenPreviewResolved', 'previewResolved'] and event['ready'] and event['resourceUrl']:
            assert event.get('requestedAssetId', event['assetId']) == event['assetId'], 'Preview response asset differs from request.'
            assert event['assetId'] in names
            assert event['resourceUrl'] not in handles or handles[event['resourceUrl']] == event['assetId']
            handles[event['resourceUrl']] = event['assetId']
            variant = event.get('variant', 'screen')
            assert event.get('requestedVariant', variant) == variant, 'Preview response quality differs from request.'
            assert variant in ['small', 'medium', 'screen', 'poster']
            assert event['resourceUrl'] not in variants or variants[event['resourceUrl']] == variant
            variants[event['resourceUrl']] = variant
    next_delivery = min((e['deliveredAt'] for e in events
                         if e['type'] == 'navigationDelivered' and e['id'] > end_id), default=float('inf'))
    frames = [e for e in events if e['type'] == 'viewerFrame' and e['at'] < next_delivery]
    frame_times = [f['at'] for f in frames]
    assert all(a <= b for a, b in zip(frame_times, frame_times[1:])), 'Frame times must be ordered.'
    focus_events = [e for e in events if e['type'] in ('blur', 'focus')]
    focus_times = [e['at'] for e in focus_events]
    assert all(a <= b for a, b in zip(focus_times, focus_times[1:])), 'Focus times must be ordered.'

    def image_state(frame):
        image = frame['image']
        if not image or not image['src']:
            return 'placeholder'
        aid = handles.get(image['src'].replace('http://media.localhost/', 'media://localhost/'))
        if aid is None:
            return 'unknown'
        if image['alt'] != frame['filename'] or names[aid] != frame['filename']:
            return 'mismatch'
        return 'decoded' if image['complete'] and image['width'] > 0 and image['height'] > 0 else 'loading'

    def image_variant(frame):
        return variants.get(frame['image']['src'].replace('http://media.localhost/', 'media://localhost/'))

    completed, missing, screen_targets, screen_times, round_trips = [], [], [], [], []
    stale_return_frames = []
    for (event, name), stop in zip(targets, (*deliveries[1:], next_delivery)):
        window = frames[bisect_left(frame_times, event['deliveredAt']):bisect_left(frame_times, stop)]

        def matches(frame):
            return (frame.get('navigationId', event['id']) == event['id'] and frame['filename'] == name
                    and frame['visibility'] == 'visible' and frame['focused'] and image_state(frame) == 'decoded')

        pair = decoded_pair(window, matches)
        if pair:
            completed.append(dict(id=event['id'], filename=name,
                                  elapsedToSecondRafMs=pair[1]['at'] - event['deliveredAt'],
                                  secondRafAt=pair[1]['at'], previewVariantAtSecondRaf=image_variant(pair[1])))
        else:
            missing.append(dict(id=event['id'], filename=name))
        screen = next((f for f in window if matches(f) and image_variant(f) == 'screen'), None)
        if screen:
            screen_targets.append(event['id'])
            screen_times.append(screen['at'])

        transitions = focus_events[bisect_left(focus_times, event['deliveredAt']):bisect_left(focus_times, stop)]
        for index, (blur, focus) in enumerate(zip(transitions, transitions[1:])):
            if not (blur['type'] == 'blur' and blur['focused'] is False and
                    focus['type'] == 'focus' and focus['focused'] is True and
                    blur['at'] < focus['at']):
                continue
            focus_end = transitions[index + 2]['at'] if index + 2 < len(transitions) else stop
            returned = [f for f in window if focus['at'] <= f['at'] < focus_end]
            stale = next((f for f in returned if f['visibility'] == 'visible' and f['focused'] and
                          (f['filename'] != name or f.get('navigationId') != event['id'])), None)
            if stale:
                stale_return_frames.append(dict(id=event['id'], expected=name, at=stale['at'],
                                                filename=stale['filename']))
                continue
            recovery = decoded_pair(returned, matches)
            if recovery:
                round_trips.append(dict(id=event['id'], filename=name, blurAt=blur['at'], focusAt=focus['at'],
                                        secondRafAt=recovery[1]['at']))
    end = frames[-1]['at'] if missing and frames else max((e['secondRafAt'] for e in completed), default=0)
    start = inputs[0]['deliveredAt'] if inputs else 0
    observed = [f for f in frames if start <= f['at'] <= end]
    mismatches = [dict(at=f['at'], filename=f['filename'], image=f['image'])
                  for f in observed if image_state(f) == 'mismatch']
    unknown = [f for f in observed if image_state(f) == 'unknown']
    foreground = bool(observed) and all(f['visibility'] == 'visible' and f['focused'] for f in observed)
    gaps = [b['at'] - a['at'] for a, b in zip(observed, observed[1:])]
    coverage = bool(observed and inputs and observed[0]['at'] - start <= 100
                    and observed[-1]['at'] >= inputs[-1]['deliveredAt']
                    and max(gaps, default=0) <= 100)
    values = sorted(e['elapsedToSecondRafMs'] for e in completed)
    ready = dict(samples=len(values), p50Ms=values[math.ceil(.5 * len(values)) - 1] if values else None,
                 p95Ms=values[math.ceil(.95 * len(values)) - 1] if values else None,
                 maxMs=max(values, default=None))
    complete_inputs = bool(inputs) and len(accepted) == len(inputs) and all(
        accepted.get(e['id']) and e['visibility'] == 'visible' and e['focused'] and not e['repeat'] for e in inputs)
    failed = any(e.get('observerFailed') for e in events if e['type'] == 'domSample')
    focus_end = max([end, *screen_times, *(trip['secondRafAt'] for trip in round_trips)])
    focus_frames = [f for f in frames if start <= f['at'] <= focus_end]
    focus_coverage = bool(round_trips and foreground_coverage(start, focus_end, focus_frames, round_trips))
    focus_photos = len({trip['filename'] for trip in round_trips})
    focus_complete = bool(complete_inputs and installed[0]['previewHandleMarksSupported'] and not failed
                          and len(targets) == len({name for _, name in targets}) == expected_focus_photos
                          and focus_photos >= required_focus_returns and focus_coverage
                          and not missing and not stale_return_frames and len(screen_targets) == len(targets)
                          and all(image_state(f) not in ('unknown', 'mismatch') for f in focus_frames))
    return dict(inputCount=len(inputs), inputIdRange=[start_id, end_id], firstPosition=first_position,
                acceptedInputs=sum(accepted.values()),
                inputDeliverySpanMs=inputs[-1]['deliveredAt'] - start if inputs else None,
                repeats=sum(bool(e['repeat']) for e in inputs), observedFrames=len(observed),
                allObservedFramesVisibleFocused=foreground, maximumRafGapMs=max(gaps, default=None),
                coverageHasNoGapOver100ms=coverage, resolvedScreenHandles=sum(v == 'screen' for v in variants.values()),
                resolvedPreviewHandles=len(handles),
                unknownHandleFrames=len(unknown), mismatchedImageFrames=mismatches,
                distinctDecodedPhotos=len({f['filename'] for f in observed if image_state(f) == 'decoded'}),
                decodedFrameVariants={v: sum(image_state(f) == 'decoded' and image_variant(f) == v for f in observed)
                                      for v in ['small', 'medium', 'screen', 'poster']},
                distinctScreenDecodedPhotos=len({f['filename'] for f in observed
                                                 if image_state(f) == 'decoded' and image_variant(f) == 'screen'}),
                placeholderFrames=sum(image_state(f) == 'placeholder' for f in observed),
                loadingFrames=sum(image_state(f) == 'loading' for f in observed),
                observerFailureSeen=failed,
                missingDecodedTargets=missing, decodedTargetSecondRaf=ready, targets=completed,
                allTargetsObservedDecoded=bool(targets) and not missing,
                screenDecodedTargetIds=screen_targets,
                allTargetsObservedScreenDecoded=bool(targets) and len(screen_targets) == len(targets),
                completeNativeDomObservation=bool(complete_inputs and installed[0]['previewHandleMarksSupported']
                    and foreground and coverage and not missing and not unknown and not mismatches and not failed),
                focusVariant=dict(expectedDistinctPhotos=expected_focus_photos, requiredReturns=required_focus_returns,
                                  distinctPhotosWithObservedReturn=focus_photos, roundTrips=round_trips,
                                  staleReturnFrames=stale_return_frames, foregroundCoverageComplete=focus_coverage,
                                  completeDomObservation=focus_complete),
                limits=['Every sampled image source is correlated with its actual preview response, variant and owned catalog ID.',
                        'A decoded small preview is not a full screen-resolution preview; quality coverage is reported separately.',
                        'Each target can only use frames before the next navigation input, even when the same photo is revisited.',
                        'The focus variant uses WebView blur/focus and requires matching decoded frames after each counted return; OS foreground must be independently verified.',
                        'RAF/source/decode observations do not prove physical pixel presentation or exclude flashes between callbacks.',
                        'Decoded timing starts at DOM key delivery; queued navigation and preview loading are included.',
                        '100ms frame-gap criterion qualifies observation coverage; it is not a product latency threshold.'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log', type=Path)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--order', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--first-position', type=int, default=1)
    parser.add_argument('--start-id', type=int, default=1)
    parser.add_argument('--end-id', type=int, default=1000000)
    parser.add_argument('--required-focus-returns', type=int, default=20)
    parser.add_argument('--expected-focus-photos', type=int, default=100)
    parser.add_argument('--os-focus-receipt', type=Path, help='Corroborate WebView focus with the same process OS trace')
    args = parser.parse_args()
    raw = args.log.read_bytes()
    rows = [json.loads(line) for line in raw.splitlines()]
    starts = [r for r in rows if r['type'] == 'nativeStart']
    assert len(starts) == 1 and all(r['runId'] == starts[0]['runId'] for r in rows)
    events = [r['event'] for r in rows if r['type'] == 'frontendBatchEvent']
    result = analyze_events(events, json.loads(args.manifest.read_text()),
                            [e['filename'] for e in json.loads(args.order.read_text())],
                            args.first_position, args.start_id, args.end_id,
                            required_focus_returns=args.required_focus_returns, expected_focus_photos=args.expected_focus_photos)
    if args.os_focus_receipt:
        os_raw = args.os_focus_receipt.read_bytes()
        result['focusVariant']['osCorroboration'] = corroborate_os_focus(
            result['focusVariant'], json.loads(os_raw)['osFocusObservation'], starts[0], events)
        result['focusVariant']['osReceiptSha256'] = hashlib.sha256(os_raw).hexdigest()
    result.update(rawFile=str(args.log.resolve()), rawSha256=hashlib.sha256(raw).hexdigest(), run=starts[0])
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k not in ['targets', 'mismatchedImageFrames', 'limits']}, ensure_ascii=False))
