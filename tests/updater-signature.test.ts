import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { test } from "node:test";
import { UpdaterSignatureError, verifyUpdaterSignatureBytes } from "../scripts/verify-updater-signature.mjs";

function syntheticSignature(artifact: Buffer) {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const rawPublicKey = publicKey.export({ format: "der", type: "spki" }).subarray(-32);
  const keyId = Buffer.from("0102030405060708", "hex");
  const signature = sign(null, createHash("blake2b512").update(artifact).digest(), privateKey);
  const trustedComment = "timestamp:1787961600\tfile:synthetic.tar.gz";
  const globalSignature = sign(null, Buffer.concat([signature, Buffer.from(trustedComment)]), privateKey);
  const publicKeyText = `untrusted comment: synthetic updater public key\n${Buffer.concat([Buffer.from("Ed"), keyId, rawPublicKey]).toString("base64")}\n`;
  const minisignText = [
    "untrusted comment: synthetic signature",
    Buffer.concat([Buffer.from("ED"), keyId, signature]).toString("base64"),
    `trusted comment: ${trustedComment}`,
    globalSignature.toString("base64"),
  ].join("\n");
  return { publicKeyText, encodedEnvelope: Buffer.from(minisignText).toString("base64") };
}

test("Node updater verifier 接受 prehashed Minisign/Ed25519 且拒绝篡改", () => {
  const artifact = Buffer.from("synthetic updater artifact");
  const fixture = syntheticSignature(artifact);
  assert.equal(verifyUpdaterSignatureBytes(artifact, fixture.encodedEnvelope, fixture.publicKeyText), true);
  assert.throws(
    () => verifyUpdaterSignatureBytes(Buffer.from("tampered updater artifact"), fixture.encodedEnvelope, fixture.publicKeyText),
    UpdaterSignatureError,
  );
  const decodedLines = Buffer.from(fixture.encodedEnvelope, "base64").toString("utf8").split("\n");
  const legacyPacket = Buffer.from(decodedLines[1], "base64");
  legacyPacket.write("Ed", 0, "ascii");
  decodedLines[1] = legacyPacket.toString("base64");
  assert.throws(
    () => verifyUpdaterSignatureBytes(artifact, Buffer.from(decodedLines.join("\n")).toString("base64"), fixture.publicKeyText),
    /legacy updater signatures are forbidden/,
  );
});
