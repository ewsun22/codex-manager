#!/usr/bin/env node

import { createHash, createPublicKey, verify } from "node:crypto";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

class UpdaterSignatureError extends Error {}

function assert(condition, message) {
  if (!condition) throw new UpdaterSignatureError(message);
}

function strictBase64(value, label) {
  assert(typeof value === "string" && value.length > 0 && value.length % 4 === 0, `${label} is not canonical base64`);
  assert(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value), `${label} is not canonical base64`);
  const decoded = Buffer.from(value, "base64");
  assert(decoded.toString("base64") === value, `${label} is not canonical base64`);
  return decoded;
}

function parsePublicKey(text) {
  const lines = text.replace(/\n$/, "").split("\n");
  assert(lines.length === 2 && lines[0].startsWith("untrusted comment: "), "updater public key envelope is invalid");
  const packet = strictBase64(lines[1], "updater public key");
  assert(packet.length === 42, "updater public key packet length is invalid");
  const algorithm = packet.subarray(0, 2).toString("ascii");
  assert(algorithm === "Ed" || algorithm === "ED", "updater public key algorithm is unsupported");
  return { keyId: packet.subarray(2, 10), rawKey: packet.subarray(10, 42) };
}

function parseSignatureEnvelope(encodedEnvelope) {
  const minisignText = strictBase64(encodedEnvelope.trim(), "updater signature envelope").toString("utf8");
  const lines = minisignText.replace(/\n$/, "").split("\n");
  assert(lines.length === 4 && lines[0].startsWith("untrusted comment: "), "updater signature envelope is invalid");
  const packet = strictBase64(lines[1], "updater signature packet");
  assert(packet.length === 74, "updater signature packet length is invalid");
  assert(packet.subarray(0, 2).toString("ascii") === "ED", "legacy updater signatures are forbidden");
  assert(lines[2].startsWith("trusted comment: "), "updater trusted comment is invalid");
  const globalSignature = strictBase64(lines[3], "updater global signature");
  assert(globalSignature.length === 64, "updater global signature length is invalid");
  return {
    keyId: packet.subarray(2, 10),
    signature: packet.subarray(10, 74),
    trustedComment: lines[2].slice("trusted comment: ".length),
    globalSignature,
  };
}

function verifyUpdaterSignatureBytes(artifact, encodedEnvelope, publicKeyText) {
  assert(Buffer.isBuffer(artifact), "updater artifact must be bytes");
  const publicKey = parsePublicKey(publicKeyText);
  const signature = parseSignatureEnvelope(encodedEnvelope);
  assert(publicKey.keyId.equals(signature.keyId), "updater signature key ID does not match the configured public key");
  const keyObject = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, publicKey.rawKey]),
    format: "der",
    type: "spki",
  });
  const digest = createHash("blake2b512").update(artifact).digest();
  assert(verify(null, digest, keyObject, signature.signature), "updater signature does not match the artifact");
  const globalMessage = Buffer.concat([signature.signature, Buffer.from(signature.trustedComment, "utf8")]);
  assert(verify(null, globalMessage, keyObject, signature.globalSignature), "updater trusted comment signature is invalid");
  return true;
}

async function verifyUpdaterSignature(artifactPath, signaturePath, publicKeyPath) {
  const [artifact, encodedEnvelope, publicKeyText] = await Promise.all([
    readFile(artifactPath),
    readFile(signaturePath, "utf8"),
    readFile(publicKeyPath, "utf8"),
  ]);
  return verifyUpdaterSignatureBytes(artifact, encodedEnvelope, publicKeyText);
}

async function main() {
  if (process.argv.length !== 5) {
    throw new UpdaterSignatureError("usage: verify-updater-signature.mjs ARTIFACT SIGNATURE PUBLIC_KEY");
  }
  await verifyUpdaterSignature(process.argv[2], process.argv[3], process.argv[4]);
  process.stdout.write("updater-signature=valid\n");
}

export { UpdaterSignatureError, parsePublicKey, parseSignatureEnvelope, verifyUpdaterSignature, verifyUpdaterSignatureBytes };

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`updater signature verification failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
