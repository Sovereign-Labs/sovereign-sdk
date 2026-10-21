import * as ed25519 from "@noble/ed25519";
import { describe, expect, it } from "vitest";
import { CommonwareSigner } from "./commonware";
import { Ed25519Signer } from "./ed25519";

describe("CommonwareSigner", () => {
  it.each([{ bytes: [] }, { bytes: [4, 5, 6] }, { bytes: [0, 4, 5, 6] }])(
    "adds exactly one prefix without changing the underlying signer: $bytes",
    async ({ bytes }) => {
      const rawSigner = new Ed25519Signer(new Uint8Array(32).fill(1));
      const signer = new CommonwareSigner(rawSigner);
      const message = new Uint8Array(bytes);
      const originalMessage = message.slice();
      const publicKey = await signer.publicKey();
      const signature = await signer.sign(message);

      expect(publicKey).toEqual(await rawSigner.publicKey());
      expect(message).toEqual(originalMessage);
      expect(
        await ed25519.verifyAsync(
          signature,
          new Uint8Array([0, ...message]),
          publicKey,
        ),
      ).toBe(true);
      expect(await ed25519.verifyAsync(signature, message, publicKey)).toBe(
        false,
      );
      expect(
        await ed25519.verifyAsync(
          signature,
          new Uint8Array([0, 0, ...message]),
          publicKey,
        ),
      ).toBe(false);
      expect(
        await ed25519.verifyAsync(
          await rawSigner.sign(message),
          message,
          publicKey,
        ),
      ).toBe(true);
    },
  );
});
