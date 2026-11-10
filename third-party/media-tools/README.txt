Travel Gallery uses the separate FFmpeg/FFprobe command-line programs from FFmpeg
9.0.2, licensed under LGPL version 2.1 or later. FFmpeg is not owned by Travel Gallery.
The application communicates through a child process; it does not link FFmpeg libraries.

The application's Contents/Resources/media-tools directory contains:
- ffmpeg-9.0.2.tar.xz: complete corresponding, unmodified upstream source
- its detached signature, official release key and verification record
- COPYING.LGPLv2.1 and the upstream LICENSE.md (other license texts describe disabled components)
- build-info.json: exact configure arguments, compiler, target and binary/source checksums
- changes.diff: empty, because upstream source code was not modified
- BUILD.txt: standalone configure/make instructions; build-media-tools.mjs,
  media-tools-validation.mjs and profile.json: build recipe
- Windows builds also contain windows-toolchain/ runtime notices and exact installed
  package versions/license declarations in build-info.json

Rebuild on native macOS 13+ with Xcode Command Line Tools, Node.js 22+ and make:
  node scripts/build-media-tools.mjs
The source checksum and configuration are pinned in third-party/media-tools/profile.json.
For Windows x64, install MSYS2 UCRT64 and its make, diffutils, curl, tar,
mingw-w64-ucrt-x86_64-{gcc,nasm,pkgconf,zlib} packages. Set
TRAVEL_GALLERY_MSYS2_ROOT to the absolute MSYS2 root when it is not C:\msys64,
then run the same Node.js build command from Windows. The tools are native PE
executables; non-system DLL dependencies are rejected before bundling. Runtime
license texts are retained and included in the application's notice inventory.
Use TRAVEL_GALLERY_FFMPEG_ARCHIVE to provide the matching source archive without network access.
Build scripts download only at build time; the installed application never downloads tools.

GPL, version-3, nonfree and automatic external-library detection are disabled.
Only the file protocol is enabled. The binaries link only system dynamic libraries.
You may replace either separate executable next to the main executable (Contents/MacOS
on macOS) with a compatible modified version. For a locally modified application, restore its macOS code-signing integrity
seal as needed. No restriction on reverse engineering FFmpeg for debugging modifications
is imposed. See the full accompanying license for your rights.

Official source: https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz
Windows build references: https://ffmpeg.org/platform.html#Windows
MSYS2 environment: https://www.msys2.org/docs/environments/
License information: https://ffmpeg.org/legal.html
The local review package is not a commercial distribution clearance. Codec patent,
jurisdictional and other release reviews remain tracked in docs/COMPLETION-AUDIT.md.
