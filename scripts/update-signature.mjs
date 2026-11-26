import { createHash, createPublicKey, verify } from 'node:crypto';

// Tauri's base64-wrapped Minisign format: verify both the payload signature and
// the signed trusted comment. Bind the announced version to that comment too.
// https://v2.tauri.app/plugin/updater/#signing-updates
export function verifyUpdateSignature(bytes, signature, publicKey, version) {
  try {
    const keyLines = Buffer.from(publicKey.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
    const lines = Buffer.from(signature.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
    const key = Buffer.from(keyLines[1], 'base64');
    const sig = Buffer.from(lines[1], 'base64');
    const global = Buffer.from(lines[3], 'base64');
    if (
      key.length !== 42 ||
      sig.length !== 74 ||
      global.length !== 64 ||
      !['Ed', 'ED'].includes(key.subarray(0, 2).toString()) ||
      !['Ed', 'ED'].includes(sig.subarray(0, 2).toString()) ||
      !key.subarray(2, 10).equals(sig.subarray(2, 10)) ||
      !lines[2].startsWith('trusted comment: ')
    )
      throw new Error();
    const ed25519 = createPublicKey({
      key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), key.subarray(10)]),
      format: 'der',
      type: 'spki',
    });
    const message =
      sig.subarray(0, 2).toString() === 'ED' ? createHash('blake2b512').update(bytes).digest() : bytes;
    const trusted = lines[2].slice('trusted comment: '.length);
    if (
      !verify(null, message, ed25519, sig.subarray(10)) ||
      !verify(null, Buffer.concat([sig.subarray(10), Buffer.from(trusted)]), ed25519, global) ||
      trusted.split('\t').find((part) => part.startsWith('version:')) !== `version:${version}`
    )
      throw new Error();
  } catch {
    throw new Error(
      'Updater signature verification failed (key, payload, trusted comment or version mismatch).',
    );
  }
}
