"""Summarize raw benchmark observer logs without treating missing samples as passes."""
import argparse
import collections
import hashlib
import json
import math
from pathlib import Path


def distribution(values):
    values = sorted(values)
    if not values:
        return {'samples': 0, 'p50Ms': None, 'p95Ms': None, 'maxMs': None}
    return dict(samples=len(values), p50Ms=values[math.ceil(.5 * len(values)) - 1],
                p95Ms=values[math.ceil(.95 * len(values)) - 1], maxMs=values[-1])


def analyze(path, start_id=1, end_id=1000000, launch_request=None):
    raw = path.read_bytes()
    rows = [json.loads(line) for line in raw.splitlines()]
    native = [row for row in rows if row.get('type') == 'nativeStart']
    assert len(native) == 1
    assert all(row.get('runId') == native[0]['runId'] for row in rows)
    batches = [row for row in rows if row.get('type') == 'frontendBatchEvent']
    events = [row['event'] for row in batches]
    installed = [event for event in events if event['type'] == 'probeInstalled']
    assert len(installed) == 1
    ready = [row for row in batches if row['event']['type'] == 'libraryInteractiveOpportunity']
    startup = None
    if ready:
        assert len(ready) == 1
        event = ready[0]['event']
        epoch = installed[0]['timeOrigin'] + event['at']
        startup = dict(indexedCount=event.get('indexedCount'), visibility=event['visibility'], focused=event['focused'],
                       nativeMainToReceiptUpperBoundMs=ready[0]['nativeReceivedMs'],
                       frontendReadyEpochMs=epoch,
                       externalLaunchRequestToReadyMs=None if launch_request is None else epoch - launch_request,
                       scope='First library card DOM at RAF, before physical screen presentation. Native receipt is a conservative bound starting at main entry; external launch request uses the same system wall clock.')
    key_events = [event for event in events if event['type'].startswith('key') and start_id <= event.get('id', 0) <= end_id]
    delivered = [event for event in key_events if event['type'] == 'keyDelivered']
    paints = [event for event in key_events if event['type'] == 'keyPaintOpportunity']
    assert len({event['id'] for event in delivered}) == len(delivered)
    assert len({event['id'] for event in paints}) == len(paints)
    by_id = {event['id']: event for event in delivered}
    eligible, rejected = [], []
    for event in paints:
        key = by_id[event['id']]
        reasons = []
        for record in [key, event]:
            if record['visibility'] != 'visible':
                reasons.append('hidden')
            if not record['focused']:
                reasons.append('unfocused')
        if not event.get('stillMatching'):
            reasons.append('superseded before second RAF')
        if reasons:
            rejected.append(dict(id=event['id'], reasons=sorted(set(reasons)), elapsedMs=event['elapsedMs']))
        else:
            eligible.append(event['elapsedMs'])
    terminal_types = ['keyPaintOpportunity', 'keyNotHandled', 'keyAlreadyInState', 'keyRenderTimeout']
    terminal = collections.Counter(event['id'] for event in key_events if event['type'] in terminal_types)
    assert all(value == 1 for value in terminal.values()), 'An input has multiple terminal outcomes.'
    missing = sorted(set(by_id) - set(terminal))
    keyboard = dict(inputCount=len(delivered), inputIdRange=[start_id, end_id],
                    eventCounts=dict(collections.Counter(event['type'] for event in key_events)),
                    inputDeliverySpanMs=None if not delivered else delivered[-1]['deliveredAt'] - delivered[0]['deliveredAt'],
                    unaccountedInputIds=missing, rejectedPaintSamples=rejected,
                    visibleFocusedPaintOpportunity=distribution(eligible),
                    allInputsHaveVisibleMatchingFeedback=len(delivered) > 0 and len(eligible) == len(delivered),
                    scope='DOM delivery → matching attributes at RAF → following RAF; no claim about OS input delivery or physical pixels. Persistence must be independently checked in SQLite after exit.')
    dom = [event for event in events if event['type'] == 'domSample']
    segments = []
    for start in [event for event in events if event['type'] == 'segmentStart']:
        end = next((event for event in events if event['type'] == 'segmentEnd' and event['id'] == start['id']), None)
        until = end['at'] if end else events[-1]['at']
        frames = [event for event in events if event['type'] == 'frame' and event['segmentId'] == start['id']]
        scrolls = [event for event in events if event['type'] == 'viewportScroll' and start['at'] <= event['at'] <= until]
        samples = [event for event in dom if start['at'] <= event['at'] <= until]
        span = frames[-1]['timestamp'] - frames[0]['timestamp'] if len(frames) > 1 else 0
        intervals = [event['intervalMs'] for event in frames if event['intervalMs'] is not None]
        first_frame_gap = frames[0]['at'] - start['at'] if frames else None
        last_frame_gap = until - frames[-1]['at'] if frames else None
        first_scroll_gap = scrolls[0]['at'] - start['at'] if scrolls else None
        last_scroll_gap = until - scrolls[-1]['at'] if scrolls else None
        longest_scroll_gap = max([b['at'] - a['at'] for a, b in zip(scrolls, scrolls[1:])], default=None)
        complete_window = bool(end and frames and 59000 <= until - start['at'] <= 61000
                               and span >= 59000 and first_frame_gap <= 1000 and last_frame_gap <= 1000)
        continuous_scroll = bool(complete_window and len(scrolls) >= 60
                                 and first_scroll_gap <= 1000 and last_scroll_gap <= 1000
                                 and longest_scroll_gap is not None and longest_scroll_gap <= 1000)
        long_tasks = [event for event in events if event['type'] == 'longTask' and start['at'] <= event['startTime'] <= until]
        segments.append(dict(id=start['id'], ended=end is not None, durationMs=until - start['at'],
            frames=len(frames), callbackFps=(len(frames) - 1) * 1000 / span if span else None,
            frameCoverageMs=span, firstFrameGapMs=first_frame_gap, lastFrameToEndGapMs=last_frame_gap,
            complete60SecondFrameWindow=complete_window,
            frameIntervals=distribution(intervals), rafIntervalsOver50ms=sum(value > 50 for value in intervals),
            allFramesVisibleFocused=bool(frames) and all(event['visibility'] == 'visible' and event['focused'] for event in frames),
            viewportScrollEvents=len(scrolls), scrollTopChanges=len({event['scrollTop'] for event in scrolls}),
            longestScrollEventGapMs=longest_scroll_gap,
            firstScrollGapMs=first_scroll_gap, lastScrollToEndGapMs=last_scroll_gap,
            continuousScrollInputObserved=continuous_scroll,
            domElementsPeak=max((event['allElements'] for event in samples), default=None),
            mediaCardsPeak=max((event['cards'] for event in samples), default=None),
            indexedCounts=sorted({event.get('indexedCount') for event in samples if event.get('indexedCount')}),
            longTaskCount=len(long_tasks) if installed[0]['longTaskSupported'] else None,
            longTaskSupported=installed[0]['longTaskSupported']))
    return dict(file=str(path.absolute()), sha256=hashlib.sha256(raw).hexdigest(), run=native[0],
        percentileMethod='nearest rank: sorted[ceil(q*n)-1]', startup=startup, keyboard=keyboard, segments=segments,
        observerFailureSeen=any(event.get('observerFailed') for event in dom),
        limitations=['RAF intervals are WebView callbacks, not a compositor FPS trace.',
                     'Unsupported Long Tasks API yields null, not zero; RAF gaps are a separate proxy.',
                     'Hidden/unfocused/superseded samples are excluded and counts retained.',
                     'Continuous input qualification requires a complete 59–61s frame window, at least 60 scroll events, and no >1s edge/inter-event input gaps; it is not compositor FPS acceptance.',
                     'A four-item probe smoke run is not 30k startup or 50k scrolling acceptance.'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--start-id', type=int, default=1)
    parser.add_argument('--end-id', type=int, default=1000000)
    parser.add_argument('--launch-request-epoch-ms', type=int)
    args = parser.parse_args()
    result = analyze(args.log, args.start_id, args.end_id, args.launch_request_epoch_ms)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(dict(startup=result['startup'], keyboard=result['keyboard']), ensure_ascii=False))
