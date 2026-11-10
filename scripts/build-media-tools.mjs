// Build separate LGPL-only command-line tools; the application never downloads tools at runtime.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  validateSourcePaths,
  validateLocalOnlyTool,
  windowsSystemImports,
} from './media-tools-validation.mjs';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const profile = JSON.parse(readFileSync(path.join(root, 'third-party/media-tools/profile.json'), 'utf8'));
const windows = process.platform === 'win32';
if (
  !(windows
    ? process.arch === 'x64'
    : process.platform === 'darwin' && ['arm64', 'x64'].includes(process.arch))
)
  throw new Error('Media-tool builds require native macOS arm64/x64 or Windows x64.');
const target = windows
  ? 'x86_64-pc-windows-msvc'
  : process.arch === 'arm64'
    ? 'aarch64-apple-darwin'
    : 'x86_64-apple-darwin';
const requested = process.env.TAURI_ENV_TARGET_TRIPLE ?? process.env.CARGO_BUILD_TARGET ?? target;
if (requested !== target)
  throw new Error('Cross-compiling media tools is not supported; use the native target machine.');
const cache = path.join(homedir(), '.cache/travel-gallery/media-tools', profile.version, target);
mkdirSync(cache, { recursive: true });
const digest = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const recipeHash = createHash('sha256')
  .update(readFileSync(fileURLToPath(import.meta.url)))
  .update(readFileSync(path.join(root, 'scripts/media-tools-validation.mjs')))
  .digest('hex');
let bash;
const msysEnv = { ...process.env, MSYSTEM: 'UCRT64', CHERE_INVOKING: 'yes', MSYS2_PATH_TYPE: 'minimal' };
if (windows) {
  const msysRoot =
    process.env.TRAVEL_GALLERY_MSYS2_ROOT || path.join(process.env.SystemDrive || 'C:', 'msys64');
  if (!path.isAbsolute(msysRoot))
    throw new Error('TRAVEL_GALLERY_MSYS2_ROOT must be an absolute MSYS2 installation path.');
  bash = path.join(msysRoot, 'usr/bin/bash.exe');
  if (!existsSync(bash) || !existsSync(path.join(msysRoot, 'ucrt64/bin/gcc.exe')))
    throw new Error(
      'MSYS2 UCRT64 is required. See docs/MEDIA-TOOLS.md for build prerequisites; the application does not install tools.',
    );
}
const posixPaths = new Map();
function posixPath(value) {
  if (!windows || !path.isAbsolute(value)) return value;
  if (!posixPaths.has(value))
    posixPaths.set(
      value,
      execFileSync(bash, ['-lc', 'cygpath -u -- "$1"', 'travel-gallery', value], {
        env: msysEnv,
        encoding: 'utf8',
      }).trim(),
    );
  return posixPaths.get(value);
}
function execute(command, args, options = {}) {
  if (!windows) return execFileSync(command, args, options);
  // Arguments remain positional; paths, spaces and metacharacters never become shell code.
  const { cwd = root, ...rest } = options;
  return execFileSync(
    bash,
    [
      '-lc',
      'cd -- "$1" && shift && exec "$@"',
      'travel-gallery',
      posixPath(cwd),
      command,
      ...args.map(posixPath),
    ],
    { env: msysEnv, ...rest },
  );
}
const run = (command, args, options = {}) => execute(command, args, { stdio: 'inherit', ...options });
const output = (command, args, options = {}) => execute(command, args, { encoding: 'utf8', ...options });
const binaryName = (name) => `${name}${windows ? '.exe' : ''}`;
const infoPath = path.join(cache, 'build-info.json');
let info = existsSync(infoPath) ? JSON.parse(readFileSync(infoPath, 'utf8')) : null;
const profileHash = digest(path.join(root, 'third-party/media-tools/profile.json'));
const valid =
  info?.profileSha256 === profileHash &&
  info?.target === target &&
  (!windows ||
    (info.recipeSha256 === recipeHash &&
      info.toolchainNotices?.length > 0 &&
      info.toolchainNotices.every(
        (notice) =>
          existsSync(path.join(cache, notice.file)) &&
          digest(path.join(cache, notice.file)) === notice.sha256,
      ))) &&
  existsSync(path.join(cache, profile.archive)) &&
  digest(path.join(cache, profile.archive)) === profile.sha256 &&
  ['ffmpeg', 'ffprobe'].every(
    (name) =>
      existsSync(path.join(cache, binaryName(name))) &&
      digest(path.join(cache, binaryName(name))) === info.binaries[name],
  );
if (!valid) {
  const archive = path.join(cache, profile.archive);
  if (!existsSync(archive) || digest(archive) !== profile.sha256) {
    const partial = path.join(cache, `${profile.archive}.part`);
    if (process.env.TRAVEL_GALLERY_FFMPEG_ARCHIVE)
      copyFileSync(process.env.TRAVEL_GALLERY_FFMPEG_ARCHIVE, partial);
    else
      run('/usr/bin/curl', [
        '--fail',
        '--location',
        '--proto',
        '=https',
        '--proto-redir',
        '=https',
        '--retry',
        '2',
        '--max-time',
        '240',
        profile.source,
        '--output',
        partial,
      ]);
    if (digest(partial) !== profile.sha256)
      throw new Error('FFmpeg source checksum mismatch; no source code executed.');
    renameSync(partial, archive);
  }
  const work = mkdtempSync(path.join(cache, 'build-'));
  validateSourcePaths(output('/usr/bin/tar', ['-tf', archive]), profile.version);
  run('/usr/bin/tar', ['-xf', archive, '-C', work]);
  const source = path.join(work, `ffmpeg-${profile.version}`);
  const flags = [
    ...profile.configure,
    ...(windows
      ? [
          '--arch=x86_64',
          '--target-os=mingw32',
          '--cc=/ucrt64/bin/gcc',
          '--cxx=/ucrt64/bin/g++',
          '--disable-pthreads',
          '--enable-w32threads',
          '--disable-hwaccels',
          '--disable-dxva2',
          '--disable-d3d11va',
          '--disable-d3d12va',
          '--disable-mediafoundation',
          '--pkg-config=/ucrt64/bin/pkg-config',
          '--pkg-config-flags=--static',
          '--extra-ldflags=-static',
        ]
      : [
          '--cc=/usr/bin/clang',
          '--extra-cflags=-mmacosx-version-min=13.0',
          '--extra-ldflags=-mmacosx-version-min=13.0',
        ]),
  ];
  if (windows) run('/usr/bin/bash', ['./configure', ...flags], { cwd: source });
  else run(path.join(source, 'configure'), flags, { cwd: source });
  run('/usr/bin/make', ['-j4', ...['ffmpeg', 'ffprobe'].map(binaryName)], { cwd: source });
  const binaries = {};
  const dependencies = {};
  for (const name of ['ffmpeg', 'ffprobe']) {
    const built = path.join(source, binaryName(name));
    run(windows ? '/ucrt64/bin/strip' : '/usr/bin/strip', [windows ? '--strip-unneeded' : '-S', built]);
    if (windows) dependencies[name] = windowsSystemImports(output('/ucrt64/bin/objdump', ['-p', built]));
    else {
      const libraries = output('/usr/bin/otool', ['-L', built]);
      if (
        libraries
          .split('\n')
          .slice(1)
          .some((line) => line.trim() && !/^\s*\/(usr\/lib|System\/Library)\//.test(line))
      )
        throw new Error(`Non-system dynamic dependency in ${name}: ${libraries}`);
    }
    const license = execFileSync(built, ['-L'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    const protocols = execFileSync(built, ['-protocols'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, PATH: '' },
    });
    validateLocalOnlyTool(license, protocols);
    copyFileSync(built, path.join(cache, binaryName(name)));
    binaries[name] = digest(built);
  }
  info = {
    version: profile.version,
    target,
    source: profile.source,
    sourceSha256: profile.sha256,
    profileSha256: profileHash,
    sourceChanges: 'none',
    configure: flags,
    binaries,
    compiler: output(windows ? '/ucrt64/bin/gcc' : '/usr/bin/clang', ['--version']).trim(),
    builtAt: new Date().toISOString(),
    license: 'LGPL-2.1-or-later',
    networkProtocols: 'disabled; local file protocol only',
    ...(windows
      ? {
          recipeSha256: recipeHash,
          nativeWindowsExecutionVerified: true,
          systemDllImports: dependencies,
          toolchain:
            'MSYS2 UCRT64 GCC; static tool libraries; Tauri sidecar target names use the Rust MSVC target',
        }
      : { minimumMacOS: '13.0' }),
  };
  if (windows) {
    // MSYS2 has split gcc-libs into smaller packages. Resolve installed file
    // owners instead of assuming a particular package layout or release date.
    info.staticRuntimeInputs = [];
    for (const name of ['libgcc.a', 'libmingwex.a', 'libz.a', 'libwinpthread.a']) {
      const library = output('/ucrt64/bin/gcc', [`-print-file-name=${name}`]).trim();
      if (library === name) throw new Error(`Missing UCRT64 static runtime library: ${name}`);
      const native = output('/usr/bin/cygpath', ['-w', library]).trim();
      if (!statSync(native).isFile()) throw new Error(`Missing static runtime input: ${library}`);
      const owner = output('/usr/bin/pacman', ['-Qqo', library]).trim();
      if (!/^mingw-w64-ucrt-x86_64-[a-zA-Z0-9+_.-]+$/.test(owner))
        throw new Error(`Unexpected UCRT64 runtime owner: ${owner}`);
      info.staticRuntimeInputs.push({ library: name, path: library, package: owner });
    }
    const packages = [...new Set(info.staticRuntimeInputs.map((item) => item.package))];
    info.toolchainPackages = output('/usr/bin/pacman', [
      '-Q',
      'mingw-w64-ucrt-x86_64-gcc',
      ...packages,
    ]).trim();
    info.toolchainNotices = [];
    info.runtimePackages = [];
    for (const pkg of packages) {
      const metadata = output('/usr/bin/pacman', ['-Qi', pkg], { env: { ...msysEnv, LC_ALL: 'C' } });
      const version = /^Version\s*:\s*(.+)$/m.exec(metadata)?.[1]?.trim();
      const declaration = /^Licenses\s*:\s*([^\r\n]+)((?:\r?\n[ \t]+[^\r\n]+)*)/m.exec(metadata);
      if (!version || !declaration) throw new Error(`Missing installed package version/license for ${pkg}.`);
      info.runtimePackages.push({
        name: pkg,
        version,
        license: `${declaration[1]} ${declaration[2]}`.replace(/\s+/g, ' ').trim(),
        noticeDirectory: path.posix.join('windows-toolchain', pkg),
        metadata,
      });
      const entries = output('/usr/bin/pacman', ['-Ql', pkg]).trim().split(/\r?\n/);
      const count = info.toolchainNotices.length;
      for (const entry of entries) {
        const installed = entry.slice(entry.indexOf(' ') + 1);
        if (!installed.startsWith('/ucrt64/share/licenses/') || installed.endsWith('/')) continue;
        const native = output('/usr/bin/cygpath', ['-w', installed]).trim();
        if (!statSync(native).isFile()) continue;
        // Flatten under the owning package; notice inventory reads COPYING files at this root.
        const relative = path.posix.join(
          'windows-toolchain',
          pkg,
          `COPYING.${info.toolchainNotices.length}.${path.posix.basename(installed)}`,
        );
        const destination = path.join(cache, relative);
        mkdirSync(path.dirname(destination), { recursive: true });
        copyFileSync(native, destination);
        info.toolchainNotices.push({ package: pkg, file: relative, sha256: digest(destination) });
      }
      if (count === info.toolchainNotices.length)
        throw new Error(`Missing installed runtime license texts for ${pkg}; refusing incomplete resources.`);
    }
  }
  for (const name of ['COPYING.LGPLv2.1', 'LICENSE.md', 'COPYING.GPLv2', 'COPYING.GPLv3', 'COPYING.LGPLv3'])
    copyFileSync(path.join(source, name), path.join(cache, name));
  writeFileSync(infoPath, JSON.stringify(info, null, 2) + '\n');
}
const resources = path.join(root, 'src-tauri/resources/media-tools');
mkdirSync(resources, { recursive: true });
mkdirSync(path.join(root, 'src-tauri/binaries'), { recursive: true });
for (const name of ['ffmpeg', 'ffprobe'])
  copyFileSync(
    path.join(cache, binaryName(name)),
    path.join(root, 'src-tauri/binaries', `${name}-${target}${windows ? '.exe' : ''}`),
  );
if (windows)
  cpSync(path.join(cache, 'windows-toolchain'), path.join(resources, 'windows-toolchain'), {
    recursive: true,
  });
for (const name of [
  profile.archive,
  'COPYING.LGPLv2.1',
  'LICENSE.md',
  'COPYING.GPLv2',
  'COPYING.GPLv3',
  'COPYING.LGPLv3',
  'build-info.json',
])
  copyFileSync(path.join(cache, name), path.join(resources, name));
for (const name of ['source-verification.json', 'ffmpeg-devel.asc', 'ffmpeg-9.0.2.tar.xz.asc', 'README.txt'])
  copyFileSync(path.join(root, 'third-party/media-tools', name), path.join(resources, name));
copyFileSync(path.join(root, 'scripts/build-media-tools.mjs'), path.join(resources, 'build-media-tools.mjs'));
copyFileSync(
  path.join(root, 'scripts/media-tools-validation.mjs'),
  path.join(resources, 'media-tools-validation.mjs'),
);
copyFileSync(path.join(root, 'third-party/media-tools/profile.json'), path.join(resources, 'profile.json'));
writeFileSync(
  path.join(resources, 'BUILD.txt'),
  `Unmodified FFmpeg ${profile.version}; ${windows ? 'MSYS2 UCRT64 GCC, make, NASM, pkgconf, zlib required' : 'Xcode Command Line Tools and make required'}.\n` +
    `Extract ${profile.archive}, then change into ffmpeg-${profile.version} and run:\n\n` +
    `./configure ${info.configure.map((flag) => `'${flag}'`).join(' ')}\n` +
    `/usr/bin/make -j4 ${['ffmpeg', 'ffprobe'].map(binaryName).join(' ')}\n\n` +
    'No upstream source changes. build-media-tools.mjs and profile.json record the application build recipe.\n' +
    'For the repository build command, preserve scripts/ and third-party/media-tools/ paths as described in README.txt.\n',
);
writeFileSync(path.join(resources, 'changes.diff'), '');
console.log(
  `Prepared FFmpeg/FFprobe ${profile.version} for ${target}, with matching unmodified source archive. No runtime download.`,
);
