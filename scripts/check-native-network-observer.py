"""Calibrate macOS nettop or sample the verified executable and its WebKit processes."""
import argparse
import csv
import ctypes
import errno
import json
from pathlib import Path
import select
import socket
import subprocess
import sys
import uuid


def nettop_rows(pids):
    targets = [part for pid in pids for part in ('-p', str(pid))]
    result = subprocess.run(
        ['/usr/bin/nettop', '-n', '-L', '1', '-J', 'interface,state,bytes_in,bytes_out', *targets],
        capture_output=True, text=True, check=True, timeout=8,
    )
    if result.stderr.strip():
        raise RuntimeError(result.stderr.strip())
    rows = list(csv.reader(result.stdout.splitlines()))
    if not rows or not {'interface', 'state', 'bytes_in', 'bytes_out'}.issubset(rows[0]):
        raise RuntimeError('Unexpected nettop header')
    fields = ['label', *rows[0][1:]]
    return [dict(zip(fields, row)) for row in rows[1:] if row]


def calibrate():
    positive = """
import socket, sys
with socket.create_connection(('127.0.0.1', int(sys.argv[1])), timeout=3) as client:
    client.sendall(sys.argv[2].encode())
    with client.makefile('rb') as stream:
        assert stream.read(len(sys.argv[2])) == sys.argv[2].encode()
    print('echo-ok', flush=True)
    sys.stdin.readline()
"""
    negative = """
import errno, json, socket, sys
try:
    socket.create_connection(('127.0.0.1', int(sys.argv[1])), timeout=3).close()
except OSError as error:
    print(json.dumps(error.errno))
    sys.exit(0 if error.errno == errno.EPERM else 1)
raise SystemExit(1)
"""
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        listener.listen(1)
        listener.settimeout(4)
        port = listener.getsockname()[1]
        nonce = uuid.uuid4().hex
        with subprocess.Popen([sys.executable, '-c', positive, str(port), nonce],
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as client:
            try:
                with listener.accept()[0] as connection:
                    connection.settimeout(4)
                    with connection.makefile('rb') as stream:
                        received = stream.read(len(nonce))
                    assert received == nonce.encode()
                    connection.sendall(received)
                    assert select.select([client.stdout], [], [], 4)[0]
                    assert client.stdout.readline().strip() == 'echo-ok'
                    peer = connection.getpeername()[1]
                    rows = nettop_rows([client.pid])
                    assert any(row['label'].endswith(f'.{client.pid}') for row in rows), rows
                    flow = next(row for row in rows if row['label'] == f'tcp4 127.0.0.1:{peer}<->127.0.0.1:{port}')
                    assert flow['interface'] == 'lo0' and flow['state'] == 'Established', flow
                    assert min(int(flow[key]) for key in ('bytes_in', 'bytes_out')) >= len(nonce), flow
                    denied = subprocess.run(
                        ['/usr/bin/sandbox-exec', '-p', '(version 1)(allow default)(deny network*)',
                         sys.executable, '-c', negative, str(port)],
                        capture_output=True, text=True, check=True, timeout=8,
                    )
                    assert json.loads(denied.stdout) == errno.EPERM
                    assert not select.select([listener], [], [], .2)[0]
                    assert client.poll() is None
            finally:
                client.stdin.close()
                try:
                    client.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    client.kill()
                    client.wait(timeout=4)
        assert client.returncode == 0
    return dict(positive=dict(loopbackEcho=True, exactProcessAndEndpointsMatched=True,
                              interface='lo0', state='Established',
                              bytesIn=int(flow['bytes_in']), bytesOut=int(flow['bytes_out'])),
                negative=dict(errno=errno.EPERM, listenerAccepted=False), ownedProcessesExited=True)


def process_snapshot():
    listing = subprocess.check_output(['ps', '-axww', '-o', 'pid=,ppid=,lstart=,comm='],
                                      text=True, errors='replace')
    return {int(parts[0]): (int(parts[1]), ' '.join(parts[2:7]), parts[7])
            for line in listing.splitlines() if len(parts := line.split(None, 7)) == 8}


def sample_webkit(executable):
    path = str(executable.resolve(strict=True))
    before = process_snapshot()
    main = [pid for pid, (_, _, command) in before.items() if command == path]
    if len(main) != 1:
        raise RuntimeError('Expected exactly one live QA executable at the supplied path')
    pid = main[0]
    # Local macOS diagnostic symbol. Availability and results must be checked
    # on the running host. This maps processes, not delegated requests.
    responsible = ctypes.CDLL(None).responsibility_get_pid_responsible_for_pid
    responsible.argtypes = [ctypes.c_int]
    responsible.restype = ctypes.c_int
    roles = {'main': pid}
    for child, (_, _, command) in before.items():
        for role in ('WebContent', 'Networking', 'GPU'):
            if command.endswith('com.apple.WebKit.' + role) and responsible(child) == pid:
                roles[f'{role}.{child}'] = child
    sockets = nettop_rows(roles.values())
    after = process_snapshot()
    if any(after.get(target) != before[target] for target in roles.values()):
        raise RuntimeError('An observed process changed identity during the socket sample')
    return dict(responsiblePidMatches=roles, socketRows=sum(row['label'].startswith(('tcp', 'udp')) for row in sockets),
                scope='Single sample of current main and associated WebKit PIDs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qa-executable', type=Path, help='Sample a previously verified isolated QA process')
    args = parser.parse_args()
    print(json.dumps(sample_webkit(args.qa_executable) if args.qa_executable else calibrate()))
