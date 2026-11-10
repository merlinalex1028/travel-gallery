"""Owned loopback-only HTTP control; serves a nonce echo, never files or user data."""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import signal
from http.server import BaseHTTPRequestHandler, HTTPServer


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
output = args.output.absolute()
assert not output.exists() and output.parent.is_dir()
# Bind before creating an ownership record: an occupied port is never reused.
server = HTTPServer(('127.0.0.1', 39841), BaseHTTPRequestHandler)
output.mkdir()
requests = []


def save(name, value):
    (output / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def respond(self, preflight=False):
        match = re.fullmatch(r'/travel-gallery-canary/([a-f0-9-]{36})', self.path)
        if not match:
            self.send_error(404)
            return
        nonce = match[1]
        payload = b'' if preflight else f'travel-gallery-canary-ok:{nonce}'.encode()
        self.send_response(204 if preflight else 200)
        self.send_header('Content-Type', 'text/plain; charset=utf-8')
        self.send_header('Content-Length', str(len(payload)))
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'GET, OPTIONS')
        self.send_header('Access-Control-Allow-Private-Network', 'true')
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(payload)
        event = dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                     method=self.command, nonce=nonce, clientIp=self.client_address[0],
                     origin=self.headers.get('Origin'), userAgent=self.headers.get('User-Agent'),
                     status=204 if preflight else 200, responseBytes=len(payload))
        requests.append(event)
        with (output / 'requests.jsonl').open('a') as log:
            log.write(json.dumps(event, ensure_ascii=False) + '\n')

    def do_GET(self):
        self.respond()

    def do_OPTIONS(self):
        self.respond(preflight=True)


server.RequestHandlerClass = Handler
save('listener.json', dict(pid=os.getpid(), endpoint='http://127.0.0.1:39841',
                          atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                          script=str(Path(__file__).resolve()), state='bound-loopback-only'))
signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(SystemExit(0)))
try:
    server.serve_forever(poll_interval=.2)
finally:
    server.server_close()
    save('listener-status.json', dict(pid=os.getpid(), state='stopped', requests=len(requests),
                                     atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat()))
