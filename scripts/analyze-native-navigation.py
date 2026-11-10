"""Correlate native viewer RAF samples with actual preview responses and owned IDs."""
import argparse
import hashlib
import json
import math
from pathlib import Path


def analyze_events(events, manifest, order, first_position=1, start_id=1, end_id=1000000):
    names = {item['id']: Path(item['relativePath']).name for item in manifest}
    assert len(set(names.values())) == len(names)
    assert set(order) == set(names.values()) and len(order) == len(names)
    installed = [e for e in events if e['type'] == 'navigationProbeInstalled']
    assert len(installed) == 1
    inputs = [e for e in events if e['type'] == 'navigationDelivered' and start_id <= e['id'] <= end_id]
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

    completed, missing = [], []
    for event, name in targets:
        pair = next(((a, b) for a, b in zip(frames, frames[1:])
                     if a['at'] >= event['deliveredAt'] and a['filename'] == b['filename'] == name
                     and all(image_state(f) == 'decoded' and f['visibility'] == 'visible' and f['focused']
                             for f in [a, b]) and b['at'] - a['at'] <= 100), None)
        if pair:
            completed.append(dict(id=event['id'], filename=name,
                                  elapsedToSecondRafMs=pair[1]['at'] - event['deliveredAt'],
                                  secondRafAt=pair[1]['at'], previewVariantAtSecondRaf=image_variant(pair[1])))
        else:
            missing.append(dict(id=event['id'], filename=name))
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
                completeNativeDomObservation=bool(complete_inputs and installed[0]['previewHandleMarksSupported']
                    and foreground and coverage and not missing and not unknown and not mismatches and not failed),
                limits=['Every sampled image source is correlated with its actual preview response, variant and owned catalog ID.',
                        'A decoded small preview is not a full screen-resolution preview; quality coverage is reported separately.',
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
    args = parser.parse_args()
    raw = args.log.read_bytes()
    rows = [json.loads(line) for line in raw.splitlines()]
    starts = [r for r in rows if r['type'] == 'nativeStart']
    assert len(starts) == 1 and all(r['runId'] == starts[0]['runId'] for r in rows)
    events = [r['event'] for r in rows if r['type'] == 'frontendBatchEvent']
    result = analyze_events(events, json.loads(args.manifest.read_text()),
                            [e['filename'] for e in json.loads(args.order.read_text())],
                            args.first_position, args.start_id, args.end_id)
    result.update(rawFile=str(args.log.resolve()), rawSha256=hashlib.sha256(raw).hexdigest(), run=starts[0])
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({k: v for k, v in result.items() if k not in ['targets', 'mismatchedImageFrames', 'limits']}, ensure_ascii=False))
