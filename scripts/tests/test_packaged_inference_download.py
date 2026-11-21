"""Range integrity and cleanup for the explicit QA model preparation path."""
from contextlib import contextmanager
import hashlib
import io
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('inference', ROOT / 'scripts/smoke-packaged-inference.py')
inference = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inference)


class PinnedDownload(unittest.TestCase):
    def setUp(self):
        self.data = b'0123456789'
        self.file = dict(path='model.safetensors', url='https://models.invalid/fixed',
                         bytes=len(self.data), sha256=hashlib.sha256(self.data).hexdigest())
        self.requests = []

    def opener(self, *, truncate=False, status=206, wrong_range=False, corrupt=False, extra=False):
        @contextmanager
        def response(request, timeout):
            start, end = map(int, request.get_header('Range').removeprefix('bytes=').split('-'))
            self.requests.append((start, end))
            data = self.data[start:end + 1]
            if corrupt:
                data = b'x' * len(data)
            if truncate and len(self.requests) == 1:
                data = data[:2]
            if extra:
                data += b'x'
            stream = io.BytesIO(data)
            stream.status = status
            stream.headers = {'Content-Range': f'bytes {start}-{end}/{len(self.data) + int(wrong_range)}'}
            yield stream
        opener = unittest.mock.Mock()
        opener.open = response
        return opener

    def run_download(self, opener, target):
        with patch.object(inference.urllib.request, 'build_opener', return_value=opener):
            inference.download_weight(target, self.file, workers=1, block_bytes=6)

    def test_short_response_resumes_at_the_first_missing_byte(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / self.file['path']
            self.run_download(self.opener(truncate=True), target)
            self.assertEqual(target.read_bytes(), self.data)
            self.assertEqual(self.requests, [(0, 5), (2, 5), (6, 9)])
            self.assertEqual(list(Path(directory).iterdir()), [target])

    def test_ignored_range_wrong_identity_and_corruption_leave_no_file(self):
        for options in (dict(status=200), dict(wrong_range=True), dict(corrupt=True), dict(extra=True)):
            with self.subTest(options=options), tempfile.TemporaryDirectory() as directory:
                with self.assertRaises(ValueError):
                    self.run_download(self.opener(**options), Path(directory) / self.file['path'])
                self.assertEqual(list(Path(directory).iterdir()), [])

    def test_an_existing_file_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / self.file['path']
            target.write_bytes(b'existing')
            with self.assertRaises(FileExistsError):
                self.run_download(self.opener(), target)
            self.assertEqual(target.read_bytes(), b'existing')
            self.assertEqual(self.requests, [])


if __name__ == '__main__':
    unittest.main()
