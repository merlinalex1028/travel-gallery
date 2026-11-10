import { execFileSync } from 'node:child_process';
import { lstatSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

export function assertFormalMacBundle(bundle) {
  if (!lstatSync(bundle).isDirectory()) throw new Error('Expected a real application directory.');
  const id = execFileSync(
    '/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleIdentifier', join(bundle, 'Contents', 'Info.plist')],
    { encoding: 'utf8' },
  ).trim();
  if (id !== 'com.travelgallery.desktop') {
    throw new Error(`Refusing to package a development or QA bundle (${id}).`);
  }
  const profileDirectories = new Set(['fixtures', 'fixture', 'thumbs', 'screenshots', 'test-results']);
  const profileFile = /\.(?:sqlite3?|db)(?:-(?:wal|shm|journal))?$|\.(?:safetensors|onnx|tflite|task)$/i;
  function check(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.isSymbolicLink() || profileDirectories.has(entry.name.toLowerCase()) || profileFile.test(entry.name)) {
        throw new Error(`Unexpected profile or test payload in the formal bundle: ${entry.name}`);
      }
      if (entry.isDirectory()) check(join(directory, entry.name));
    }
  }
  check(bundle);
}
