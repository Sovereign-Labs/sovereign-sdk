import { generateKeyPairSync, sign, verify } from "node:crypto";
import { CommonwareSigner } from "@sovereign-sdk/signers";
import { describe, expect, it, vi } from "vitest";
import { type PhantomProvider, PhantomSigner } from "./phantom";

describe("PhantomSigner", () => {
  it.each([false, true])(
    "uses Commonware framing only when wrapped: %s",
    async (commonware) => {
      const { privateKey, publicKey } = generateKeyPairSync("ed25519");
      const signMessage = vi.fn(async (message: Uint8Array) => ({
        signature: new Uint8Array(sign(null, message, privateKey)),
      }));
      const provider: PhantomProvider = {
        publicKey: null,
        connect: vi.fn(),
        disconnect: vi.fn(),
        signMessage,
      };
      const rawSigner = new PhantomSigner(provider);
      const signer = commonware ? new CommonwareSigner(rawSigner) : rawSigner;
      const message = new TextEncoder().encode('{"version":0}');
      const originalMessage = message.slice();
      const signature = await signer.sign(message);
      const framedMessage = new Uint8Array([0, ...message]);

      expect(signMessage).toHaveBeenCalledExactlyOnceWith(
        commonware ? framedMessage : message,
        "utf8",
      );
      expect(verify(null, framedMessage, publicKey, signature)).toBe(
        commonware,
      );
      expect(verify(null, message, publicKey, signature)).toBe(!commonware);
      expect(
        verify(
          null,
          new Uint8Array([0, ...framedMessage]),
          publicKey,
          signature,
        ),
      ).toBe(false);
      expect(message).toEqual(originalMessage);
    },
  );
});
