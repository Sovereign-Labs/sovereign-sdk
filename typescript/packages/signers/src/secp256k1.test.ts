import { keccak_256 } from "@noble/hashes/sha3";
import * as secp256k1 from "@noble/secp256k1";
import { describe, expect, it } from "vitest";
import { Secp256k1Signer } from "./secp256k1";

describe("Secp256k1Signer", () => {
  const testPrivateKey = new Uint8Array(32).fill(1); // Sample 32-byte private key for testing
  const testMessage = new Uint8Array([4, 5, 6]);

  it("should create signer from private key and compute public key", async () => {
    const signer = new Secp256k1Signer(testPrivateKey);
    const publicKey = await signer.publicKey();
    expect(publicKey).toBeInstanceOf(Uint8Array);
    expect(publicKey.length).toBe(33); // Compressed public key
  });

  it("should sign a message and verify the signature", async () => {
    const signer = new Secp256k1Signer(testPrivateKey);
    const signature = await signer.sign(testMessage);
    const publicKey = await signer.publicKey();
    const msgHash = keccak_256(testMessage);
    const sig = secp256k1.Signature.fromCompact(signature);
    const isValid = secp256k1.verify(sig, msgHash, publicKey);
    expect(isValid).toBe(true);
  });

  it("should fail verification with tampered message", async () => {
    const signer = new Secp256k1Signer(testPrivateKey);
    const signature = await signer.sign(testMessage);
    const publicKey = await signer.publicKey();
    const tamperedMessage = new Uint8Array([...testMessage, 7]); // Tamper by adding extra byte
    const msgHash = keccak_256(tamperedMessage);
    const sig = secp256k1.Signature.fromCompact(signature);
    const isValid = secp256k1.verify(sig, msgHash, publicKey);
    expect(isValid).toBe(false);
  });

  it("should produce 64-byte signatures", async () => {
    const signer = new Secp256k1Signer(testPrivateKey);
    const signature = await signer.sign(testMessage);
    expect(signature.length).toBe(64);
  });

  it("should handle empty messages", async () => {
    const signer = new Secp256k1Signer(testPrivateKey);
    const emptyMessage = new Uint8Array(0);
    const signature = await signer.sign(emptyMessage);
    const publicKey = await signer.publicKey();

    const msgHash = keccak_256(emptyMessage);
    const sig = secp256k1.Signature.fromCompact(signature);
    const isValid = secp256k1.verify(sig, msgHash, publicKey);
    expect(isValid).toBe(true);
  });

  it("should return compressed public keys", async () => {
    const signer = new Secp256k1Signer(testPrivateKey);

    const publicKey = await signer.publicKey();

    // Compressed public keys are 33 bytes (1 byte prefix + 32 bytes)
    expect(publicKey.length).toBe(33);

    // First byte should be 0x02 or 0x03 for compressed keys
    expect([0x02, 0x03]).toContain(publicKey[0]);
  });
});
