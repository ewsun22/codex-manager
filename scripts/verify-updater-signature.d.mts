export class UpdaterSignatureError extends Error {}
export function parsePublicKey(text: string): { keyId: Buffer; rawKey: Buffer };
export function parseSignatureEnvelope(text: string): {
  keyId: Buffer;
  signature: Buffer;
  trustedComment: string;
  globalSignature: Buffer;
};
export function verifyUpdaterSignature(artifactPath: string, signaturePath: string, publicKeyPath: string): Promise<true>;
export function verifyUpdaterSignatureBytes(artifact: Buffer, encodedEnvelope: string, publicKeyText: string): true;
