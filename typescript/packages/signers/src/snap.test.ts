import type { MetaMaskInpageProvider } from "@metamask/providers";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SignerError } from "./errors";
import { newMetaMaskSnapSigner } from "./snap";

describe("MetaMask Snap Signer", () => {
  const mockProvider = {
    request: vi.fn(),
  };

  const defaultOpts = {
    curve: "ed25519" as const,
    schema: {},
    provider: mockProvider as unknown as MetaMaskInpageProvider,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("initialization", () => {
    it("should throw if no provider is available and window.ethereum is undefined", () => {
      vi.stubGlobal("window", { ethereum: undefined });
      expect(() =>
        newMetaMaskSnapSigner({ curve: "ed25519", schema: {} }),
      ).toThrow(
        new SignerError("Failed to find provider for signer", "MetaMaskSnap"),
      );
    });
    it("should use window.ethereum if no provider is specified", async () => {
      vi.stubGlobal("window", { ethereum: mockProvider });
      const mockPublicKey = "0123456789abcdef";

      mockProvider.request.mockResolvedValueOnce({}); // wallet_getSnaps
      mockProvider.request.mockResolvedValueOnce({ publicKey: mockPublicKey });

      const signer = newMetaMaskSnapSigner({ curve: "ed25519", schema: {} });
      await signer.publicKey();

      expect(mockProvider.request).toHaveBeenCalled();
    });
    it("should prefer provided provider over window.ethereum", async () => {
      const customProvider = {
        request: vi
          .fn()
          .mockResolvedValueOnce({})
          .mockResolvedValueOnce({ publicKey: "012f" }),
      };
      vi.stubGlobal("window", { ethereum: mockProvider });

      const signer = newMetaMaskSnapSigner({
        curve: "ed25519",
        schema: {},
        provider: customProvider as unknown as MetaMaskInpageProvider,
      });
      await signer.publicKey();

      expect(window.ethereum.request).not.toHaveBeenCalled();
      expect(customProvider.request).toHaveBeenCalled();
    });

    it("should use custom snapId when provided", async () => {
      vi.stubGlobal("window", { ethereum: mockProvider });
      const signer = newMetaMaskSnapSigner({
        ...defaultOpts,
        snapId: "local:test",
      });

      mockProvider.request.mockResolvedValueOnce({});
      mockProvider.request.mockResolvedValueOnce({ publicKey: "0010" });

      await signer.publicKey();

      expect(mockProvider.request).toHaveBeenCalledWith(
        expect.objectContaining({
          params: expect.objectContaining({
            snapId: "local:test",
          }),
        }),
      );
    });
  });

  describe("publicKey", () => {
    it("should return public key successfully", async () => {
      const signer = newMetaMaskSnapSigner(defaultOpts);
      const mockPublicKey = "0123456789abcdef";

      mockProvider.request.mockResolvedValueOnce({}); // wallet_getSnaps
      mockProvider.request.mockResolvedValueOnce({ publicKey: mockPublicKey });

      const result = await signer.publicKey();

      expect(result).toEqual(
        new Uint8Array([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]),
      );
    });

    it("should throw if snap support is not available", async () => {
      const signer = newMetaMaskSnapSigner(defaultOpts);
      mockProvider.request.mockRejectedValueOnce(new Error("Method not found"));

      await expect(signer.publicKey()).rejects.toThrow(
        new SignerError(
          "Provider does not support MetaMask snaps",
          "MetaMaskSnap",
        ),
      );
    });
  });

  describe("sign", () => {
    it("should sign message successfully", async () => {
      const signer = newMetaMaskSnapSigner(defaultOpts);
      const mockSignature = "abcdef0123456789";
      const message = new Uint8Array([1, 2, 3, 4]);

      mockProvider.request.mockResolvedValueOnce({}); // wallet_getSnaps
      mockProvider.request.mockResolvedValueOnce({ signature: mockSignature });

      const result = await signer.sign(message);

      expect(result).toEqual(
        new Uint8Array([0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89]),
      );
      expect(mockProvider.request).toHaveBeenCalledWith({
        method: "wallet_invokeSnap",
        params: {
          snapId: "npm:@sovereign-sdk/metamask-snap",
          request: {
            method: "signTransaction",
            params: {
              unsignedTxHex: "01020304",
              schema: {},
              curve: "ed25519",
              path: 0,
            },
          },
        },
      });
    });

    it("should throw if snap support is not available", async () => {
      const signer = newMetaMaskSnapSigner(defaultOpts);
      mockProvider.request.mockRejectedValueOnce(new Error("Method not found"));

      await expect(signer.sign(new Uint8Array([1, 2, 3, 4]))).rejects.toThrow(
        new SignerError(
          "Provider does not support MetaMask snaps",
          "MetaMaskSnap",
        ),
      );
    });
  });
});
