import * as ed25519 from "@noble/ed25519";
import { describe, expect, it } from "vitest";
import { Ed25519Signer } from "./ed25519";

describe("Ed25519Signer", () => {
  const testPrivateKeyBytes = new Uint8Array(32).fill(1); // Sample 32-byte private key for testing
  const testMessage = new Uint8Array([4, 5, 6]);

  it("should create signer from private key bytes and compute public key", async () => {
    const signer = new Ed25519Signer(testPrivateKeyBytes);
    const publicKey = await signer.publicKey();
    expect(publicKey).toBeInstanceOf(Uint8Array);
    expect(publicKey.length).toBe(32);
  });

  it("should sign a message and verify the signature", async () => {
    const signer = new Ed25519Signer(testPrivateKeyBytes);
    const signature = await signer.sign(testMessage);
    const publicKey = await signer.publicKey();
    const isValid = await ed25519.verifyAsync(
      signature,
      testMessage,
      publicKey,
    );
    expect(isValid).toBe(true);
  });

  it("should fail verification with tampered message", async () => {
    const signer = new Ed25519Signer(testPrivateKeyBytes);
    const signature = await signer.sign(testMessage);
    const publicKey = await signer.publicKey();
    const tamperedMessage = new Uint8Array([...testMessage, 7]); // Tamper by adding extra byte
    const isValid = await ed25519.verifyAsync(
      signature,
      tamperedMessage,
      publicKey,
    );
    expect(isValid).toBe(false);
  });

  it("should produce 64-byte signatures", async () => {
    const signer = new Ed25519Signer(testPrivateKeyBytes);
    const signature = await signer.sign(testMessage);
    expect(signature.length).toBe(64);
  });

  it("should handle empty messages", async () => {
    const signer = new Ed25519Signer(testPrivateKeyBytes);
    const emptyMessage = new Uint8Array(0);
    const signature = await signer.sign(emptyMessage);
    const publicKey = await signer.publicKey();

    // Verify the signature with chain hash appended to empty message
    const isValid = await ed25519.verifyAsync(
      signature,
      emptyMessage,
      publicKey,
    );
    expect(isValid).toBe(true);
  });

  it("should produce consistent public keys", async () => {
    const signer = new Ed25519Signer(testPrivateKeyBytes);

    const publicKey1 = await signer.publicKey();
    const publicKey2 = await signer.publicKey();

    expect(publicKey1).toEqual(publicKey2);
  });
});
