import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import {
  mediaToolCaptureOptions,
  validateSourcePaths,
  windowsSystemImports,
  validateLocalOnlyTool,
} from '../media-tools-validation.mjs';

test('captures verbose PE headers beyond 1 MiB and still checks trailing DLL imports', () => {
  const dump = (dll) =>
    execFileSync(
      process.execPath,
      [
        '-e',
        'process.stdout.write("unwind table entry\\n".repeat(120000) + "\\n DLL Name: " + process.argv[1] + "\\n")',
        dll,
      ],
      mediaToolCaptureOptions,
    );
  const valid = dump('KERNEL32.dll');
  assert.ok(Buffer.byteLength(valid) > 1024 * 1024);
  assert.deepEqual(windowsSystemImports(valid), ['KERNEL32.dll']);
  assert.throws(() => windowsSystemImports(dump('libwinpthread-1.dll')), /Non-system Windows DLL/);
});

test('rejects archive traversal, Windows separators and unexpected roots before configure', () => {
  validateSourcePaths('ffmpeg-9.0.2/\nffmpeg-9.0.2/libavcodec/a.c\n', '9.0.2');
  for (const name of [
    'ffmpeg-9.0.2/../../etc/a',
    'ffmpeg-9.0.2/dir/../a',
    'ffmpeg-9.0.2/dir\\a',
    '/tmp/a',
    'ffmpeg-9.0.3/a',
    '',
  ])
    assert.throws(() => validateSourcePaths(name, '9.0.2'));
});

test('accepts system PE imports and rejects external runtime dependencies or missing evidence', () => {
  assert.deepEqual(
    windowsSystemImports(' DLL Name: KERNEL32.dll\r\n DLL Name: api-ms-win-crt-runtime-l1-1-0.dll\r\n'),
    ['KERNEL32.dll', 'api-ms-win-crt-runtime-l1-1-0.dll'],
  );
  for (const dll of [
    'zlib1.dll',
    'libgcc_s_seh-1.dll',
    'libwinpthread-1.dll',
    'msys-2.0.dll',
    'cygwin1.dll',
    'avcodec-62.dll',
    '../kernel32.dll',
  ])
    assert.throws(() => windowsSystemImports(` DLL Name: KERNEL32.dll\n DLL Name: ${dll}\n`));
  assert.throws(() => windowsSystemImports('unrecognized objdump output'));
});

test('rejects non-LGPL2.1 or network protocols in the actual tool output', () => {
  const license = 'GNU Lesser General Public License version 2.1';
  validateLocalOnlyTool(license, 'Input:\n  file\nOutput:\n  file\n');
  assert.throws(() => validateLocalOnlyTool('GNU General Public License version 3', '  file\n  file\n'));
  assert.throws(() => validateLocalOnlyTool(license, 'Input:\n  file\n  http\nOutput:\n  file\n'));
  assert.throws(() => validateLocalOnlyTool(license, ''));
});
