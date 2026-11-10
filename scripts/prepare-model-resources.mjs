// Explicit developer/user operation only. The desktop app never downloads models.
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import * as fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
export async function prepareModel(filename, commandName) {
  const profileText = await fs.readFile(path.join(root, 'src-tauri/resources/ai', filename), 'utf8');
  const profile = JSON.parse(profileText);
  const args = process.argv.slice(2);
  const modes = args.filter((arg) => ['--download', '--verify'].includes(arg));
  const mode = modes.length === 1 ? modes[0].slice(2) : null;
  const position = args.indexOf('--destination');
  if (
    !mode ||
    args.length !== 3 ||
    position < 0 ||
    !args[position + 1] ||
    args[position + 1].startsWith('--') ||
    args.some((arg, i) => i !== position + 1 && !['--download', '--verify', '--destination'].includes(arg))
  ) {
    console.log(`Usage: pnpm ${commandName} --download|--verify --destination <model-package-directory>`);
    console.log(
      `${profile.name}: ${profile.files.reduce((n, file) => n + file.bytes, 0)} bytes; ${profile.license}; ${profile.revision}`,
    );
    console.log(
      'Downloading is an explicit operation. Transfer the complete directory to an offline computer and import it in Travel Gallery.',
    );
    process.exit(args.length ? 1 : 0);
  }
  const requested = path.resolve(args[position + 1]);
  if (mode === 'download') await fs.mkdir(requested, { recursive: true });
  const destination = await fs.realpath(requested);
  async function sha256(filename) {
    const hash = createHash('sha256');
    for await (const chunk of createReadStream(filename)) hash.update(chunk);
    return hash.digest('hex');
  }
  async function verify(file, target) {
    const stat = await fs.lstat(target);
    if (
      !stat.isFile() ||
      stat.isSymbolicLink() ||
      stat.size !== file.bytes ||
      (await sha256(target)) !== file.sha256
    )
      throw new Error(`${file.path}: size, type or SHA-256 mismatch; existing files were not overwritten.`);
  }
  for (const file of profile.files.toSorted(
    (a, b) => Number(a.path === 'model.safetensors') - Number(b.path === 'model.safetensors'),
  )) {
    if (path.basename(file.path) !== file.path || !/^https:\/\//.test(file.url))
      throw new Error('Invalid bundled resource profile.');
    const target = path.join(destination, file.path);
    if (
      await fs.lstat(target).then(
        () => true,
        (error) => (error.code === 'ENOENT' ? false : Promise.reject(error)),
      )
    ) {
      await verify(file, target);
      console.log(`Verified ${file.path}`);
      continue;
    }
    if (mode !== 'download') throw new Error(`Missing ${file.path}`);
    const part = path.join(destination, `.travel-gallery-download-${randomUUID()}.part`);
    const controller = new AbortController();
    const interrupt = () => controller.abort();
    process.once('SIGINT', interrupt);
    let handle;
    try {
      console.log(`Downloading ${file.path} (${file.bytes} bytes)`);
      const response = await fetch(file.url, { signal: controller.signal });
      if (!response.ok || !response.body) throw new Error(`Model server returned HTTP ${response.status}`);
      handle = await fs.open(part, 'wx', 0o600);
      let count = 0,
        lastUpdate = Date.now();
      for await (const bytes of response.body) {
        if (count + bytes.length > file.bytes) throw new Error('Resource exceeded its pinned size.');
        let offset = 0;
        while (offset < bytes.length) {
          const written = await handle.write(bytes, offset, bytes.length - offset);
          if (!written.bytesWritten) throw new Error('Resource write made no progress.');
          offset += written.bytesWritten;
        }
        count += bytes.length;
        if (Date.now() - lastUpdate > 10000) {
          console.log(`${file.path}: ${Math.floor((count * 100) / file.bytes)}% (${count} / ${file.bytes})`);
          lastUpdate = Date.now();
        }
      }
      await handle.sync();
      await handle.close();
      handle = null;
      await verify(file, part);
      // Publishing by hard link fails if the destination appeared concurrently.
      await fs.link(part, target);
      console.log(`Verified ${file.path}`);
    } finally {
      process.removeListener('SIGINT', interrupt);
      await handle?.close();
      await fs.rm(part, { force: true });
    }
  }
  const marker = path.join(destination, 'travel-gallery-model.json');
  async function verifyMarker() {
    const stat = await fs.lstat(marker);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 65536)
      throw new Error('Model package marker must be a small regular file.');
    if (JSON.stringify(JSON.parse(await fs.readFile(marker, 'utf8'))) !== JSON.stringify(profile))
      throw new Error('Model package marker does not match the bundled profile.');
  }
  if (mode === 'verify') {
    await verifyMarker();
  } else {
    try {
      await fs.writeFile(marker, profileText, { flag: 'wx', mode: 0o600 });
    } catch (error) {
      if (error.code !== 'EEXIST') throw error;
      await verifyMarker();
    }
  }
  console.log(`Offline model package ${mode === 'verify' ? 'verified' : 'ready'}: ${destination}`);
}
