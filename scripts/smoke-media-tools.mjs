// Real metadata/poster smoke test; creates its own small video, uses no encoder or downloads.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { validateLocalOnlyTool } from './media-tools-validation.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const profile = JSON.parse(readFileSync(path.join(root, 'third-party/media-tools/profile.json'), 'utf8'));
const options = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
  assert(
    ['--tools', '--output'].includes(process.argv[i]) && process.argv[i + 1],
    'Expected --tools or --output with a path.',
  );
  assert(!options.has(process.argv[i]), 'Duplicate argument.');
  options.set(process.argv[i], path.resolve(process.argv[i + 1]));
}
const target =
  process.platform === 'win32' && process.arch === 'x64'
    ? 'x86_64-pc-windows-msvc'
    : process.platform === 'darwin' && process.arch === 'arm64'
      ? 'aarch64-apple-darwin'
      : process.platform === 'darwin' && process.arch === 'x64'
        ? 'x86_64-apple-darwin'
        : null;
assert(target, 'Native Windows x64 or macOS is required.');
const tools = options.get('--tools') ?? path.join(root, 'src-tauri/binaries');
const output = options.get('--output');
if (output) assert(!existsSync(output), 'Refusing to overwrite an earlier smoke report.');
const suffix = process.platform === 'win32' ? '.exe' : '';
const binary = (name) => {
  const normal = path.join(tools, `${name}${suffix}`);
  const selected = existsSync(normal) ? normal : path.join(tools, `${name}-${target}${suffix}`);
  assert(existsSync(selected), `Missing local tool: ${selected}`);
  return selected;
};
const env = { ...process.env, PATH: '' };
const run = (name, args) =>
  execFileSync(binary(name), args, {
    env,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    timeout: 30000,
  });
const hash = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const stamp = (file) => ({
  bytes: statSync(file).size,
  mtimeNs: statSync(file, { bigint: true }).mtimeNs.toString(),
  sha256: hash(file),
});

function chunk(type, data) {
  const size = Buffer.alloc(4);
  size.writeUInt32LE(data.length);
  return Buffer.concat([
    Buffer.from(type, 'ascii'),
    size,
    data,
    ...(data.length % 2 ? [Buffer.alloc(1)] : []),
  ]);
}
const list = (type, children) => chunk('LIST', Buffer.concat([Buffer.from(type, 'ascii'), ...children]));
// AVI RIFF / AVIMAINHEADER / AVISTREAMHEADER / BITMAPINFOHEADER, two 16x12 BGR24 frames.
// https://learn.microsoft.com/en-us/windows/win32/directshow/avi-riff-file-reference
function ownVideo() {
  const width = 16,
    height = 12,
    bytes = width * height * 3;
  const main = Buffer.alloc(56);
  [100000, bytes * 10, 0, 0, 2, 0, 1, bytes, width, height].forEach((v, i) => main.writeUInt32LE(v, i * 4));
  const stream = Buffer.alloc(56);
  stream.write('vidsDIB ', 0, 'ascii');
  [
    [20, 1],
    [24, 10],
    [32, 2],
    [36, bytes],
    [40, 0xffffffff],
  ].forEach(([offset, value]) => stream.writeUInt32LE(value, offset));
  stream.writeUInt16LE(width, 52);
  stream.writeUInt16LE(height, 54);
  const format = Buffer.alloc(40);
  format.writeUInt32LE(40, 0);
  format.writeInt32LE(width, 4);
  format.writeInt32LE(height, 8);
  format.writeUInt16LE(1, 12);
  format.writeUInt16LE(24, 14);
  format.writeUInt32LE(bytes, 20);
  const frames = [2, 1].map((channel) => {
    const pixels = Buffer.alloc(bytes);
    for (let i = channel; i < bytes; i += 3) pixels[i] = 220;
    return chunk('00db', pixels);
  });
  const headers = list('hdrl', [
    chunk('avih', main),
    list('strl', [chunk('strh', stream), chunk('strf', format)]),
  ]);
  return chunk('RIFF', Buffer.concat([Buffer.from('AVI ', 'ascii'), headers, list('movi', frames)]));
}

const versions = {};
for (const name of ['ffmpeg', 'ffprobe']) {
  versions[name] = run(name, ['-version']).split(/\r?\n/)[0];
  assert(versions[name].includes(`version ${profile.version} `));
  validateLocalOnlyTool(run(name, ['-L']), run(name, ['-protocols']));
  const rejected = spawnSync(binary(name), ['-v', 'error', '-i', 'https://127.0.0.1:9/blocked'], {
    env,
    encoding: 'utf8',
    timeout: 10000,
  });
  assert(rejected.status && rejected.stderr.includes('Protocol not found'), rejected.stderr);
}
const directory = mkdtempSync(path.join(tmpdir(), 'travel-gallery-media-📷-'));
let report;
try {
  const source = path.join(directory, '旅行 视频.avi'),
    poster = path.join(directory, '海报.jpg');
  writeFileSync(source, ownVideo());
  const before = stamp(source);
  const metadata = JSON.parse(
    run('ffprobe', ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', source]),
  );
  const video = metadata.streams.find((stream) => stream.codec_type === 'video');
  assert(video.codec_name === 'rawvideo' && video.width === 16 && video.height === 12);
  assert(Math.abs(Number(metadata.format.duration) - 0.2) < 0.001);
  run('ffmpeg', [
    '-v',
    'error',
    '-nostdin',
    '-ss',
    '0',
    '-i',
    source,
    '-frames:v',
    '1',
    '-vf',
    'scale=256:256:force_original_aspect_ratio=decrease',
    '-y',
    poster,
  ]);
  const image = JSON.parse(run('ffprobe', ['-v', 'error', '-show_streams', '-of', 'json', poster]))
    .streams[0];
  assert(image.codec_name === 'mjpeg' && image.width === 256 && image.height === 192);
  assert.deepEqual(stamp(source), before);
  report = {
    atUtc: new Date().toISOString(),
    platform: process.platform,
    target,
    scope:
      'Actual local CLI metadata/poster extraction with a generated two-frame AVI; not diverse codecs, WebView playback or Windows installer acceptance.',
    versions,
    toolSha256: Object.fromEntries(['ffmpeg', 'ffprobe'].map((name) => [name, hash(binary(name))])),
    emptyPath: true,
    onlyFileProtocol: true,
    networkProtocolRejected: true,
    unicodeAndSpacesPath: true,
    sourceUnchanged: true,
    source: before,
    durationSeconds: Number(metadata.format.duration),
    videoDimensions: [video.width, video.height],
    posterDimensions: [image.width, image.height],
    posterSha256: hash(poster),
  };
} finally {
  rmSync(directory, { recursive: true, force: true }); // Only the fresh, owned fixture directory.
}
if (output) {
  mkdirSync(path.dirname(output), { recursive: true });
  writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
}
console.log(JSON.stringify(report));
