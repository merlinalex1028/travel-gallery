"""Confirm a live isolated viewer's OS foreground role without changing the UI."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--index', type=int, required=True)
    parser.add_argument('--expect', choices=('qa', 'other'), required=True)
    parser.add_argument('--joint', action='store_true', help='Check the current joint observer batch')
    args = parser.parse_args()
    assert 1 <= args.index <= 100
    receipt = ROOT / ('docs/native-navigation-joint-latest.json' if args.joint
                      else 'docs/native-navigation-focus-latest.json')
    assert not receipt.is_symlink()
    state = json.loads(receipt.read_text())
    assert state['status'] == 'ready'
    work = Path(state['temporaryRoot']).resolve(strict=True)
    assert work.name.startswith('travel-gallery-native-navigation-focus-')
    app = Path(state['application']).resolve(strict=True)
    assert app.is_relative_to(work)
    spec = importlib.util.spec_from_file_location(
        'picker', ROOT / 'scripts/native-picker-restart-batch.py')
    picker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(picker)
    picker.b.APP_PATH = app
    pid = picker.verified_process(Path(state['expectedDatabase']))
    assert pid == state['verifiedPid']

    def ls(*arguments):
        return subprocess.check_output(
            ['/usr/bin/lsappinfo', '-nonames', *arguments],
            text=True, timeout=3).strip()

    qa = ls('find', f'pid={pid}')
    assert qa.startswith('ASN:')
    samples, streak = [], 0
    until = time.monotonic() + 5
    while time.monotonic() < until:
        front = ls('front')
        assert front.startswith('ASN:')
        role = 'qa' if front == qa else 'other'
        samples.append(dict(atUnixMs=time.time_ns() / 1_000_000, role=role))
        streak = streak + 1 if role == args.expect else 0
        if streak >= 3:
            break
        time.sleep(.15)
    # Recheck ownership before writing only to this batch's disposable control root.
    assert picker.verified_process(Path(state['expectedDatabase'])) == pid
    result = dict(index=args.index, expected=args.expect, confirmed=streak >= 3,
                  pid=pid, consecutiveSamples=streak, samples=samples)
    control = work / 'control'
    assert control.is_dir() and not control.is_symlink()
    log = control / 'focus-confirmations.jsonl'
    assert not log.is_symlink()
    with log.open('a') as output:
        output.write(json.dumps(result) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key != 'samples'}))
    return 0 if result['confirmed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
