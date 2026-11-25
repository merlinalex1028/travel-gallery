// Shared build checks. No source archive is executed before its pinned digest is checked.
// PE private headers include large unwind tables, well beyond Node's 1 MiB default.
export const mediaToolCaptureOptions = Object.freeze({
  encoding: 'utf8',
  maxBuffer: 32 * 1024 * 1024,
});

export function validateSourcePaths(listing, version) {
  const entries = listing.trim().split(/\r?\n/);
  if (
    !entries.length ||
    entries.some(
      (name) =>
        !name.startsWith(`ffmpeg-${version}/`) || name.includes('\\') || name.split('/').includes('..'),
    )
  )
    throw new Error('Unexpected source archive paths.');
}

const systemDlls = new Set([
  'advapi32.dll',
  'bcrypt.dll',
  'comdlg32.dll',
  'crypt32.dll',
  'gdi32.dll',
  'kernel32.dll',
  'msvcrt.dll',
  'normaliz.dll',
  'ntdll.dll',
  'ole32.dll',
  'oleaut32.dll',
  'psapi.dll',
  'secur32.dll',
  'shell32.dll',
  'shlwapi.dll',
  'ucrtbase.dll',
  'user32.dll',
  'version.dll',
  'winmm.dll',
  'ws2_32.dll',
]);

export function windowsSystemImports(output) {
  const imports = [...output.matchAll(/^\s*DLL Name:\s*(\S+)\s*$/gim)].map((m) => m[1]);
  if (!imports.length) throw new Error('No PE import table found.');
  for (const name of imports) {
    if (!systemDlls.has(name.toLowerCase()) && !/^(?:api|ext)-ms-win-[a-z0-9-]+\.dll$/i.test(name))
      throw new Error(`Non-system Windows DLL dependency: ${name}`);
  }
  return [...new Set(imports)].sort();
}

export function validateLocalOnlyTool(license, protocols) {
  if (!/GNU\s+Lesser\s+General\s+Public\s+License/.test(license) || !license.includes('version 2.1'))
    throw new Error('Unexpected media-tool license.');
  const names = [...protocols.matchAll(/^\s+([a-z][a-z0-9_]*)\s*$/gm)].map((m) => m[1]);
  if (names.length !== 2 || names.some((name) => name !== 'file'))
    throw new Error('Media tools must expose only local file input/output protocols.');
}
