import { beforeEach, describe, expect, it, vi } from "vitest";
import { Eip712Signer } from "./eip712";

describe("Eip712Signer", () => {
  const mockProvider = {
    request: vi.fn(),
  };

  const mockSchema = {
    types: [],
    root_type_indices: [0, 1, 2],
    chain_data: {
      chain_id: 4321,
      chain_name: "TestChain",
    },
    templates: [],
    serde_metadata: [],
  };

  const testAddress = "0x1234567890123456789012345678901234567890";

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("constructor", () => {
    it("should create a signer instance with address", () => {
      const signer = new Eip712Signer(
        mockProvider as any,
        mockSchema,
        testAddress,
      );

      expect(signer).toBeDefined();
      expect(signer.publicKey).toBeDefined();
      expect(signer.sign).toBeDefined();
    });

    it("should throw error if address is not provided", () => {
      expect(() => {
        // @ts-expect-error Testing missing address parameter
        new Eip712Signer(mockProvider as any, mockSchema);
      }).toThrow("Address is required");
    });
  });

  describe("publicKey", () => {
    it("should throw error if public key not available before signing", async () => {
      const signer = new Eip712Signer(
        mockProvider as any,
        mockSchema,
        testAddress,
      );

      await expect(signer.publicKey()).rejects.toThrow(
        "Public key was not available, you must call sign() first",
      );
    });
  });

  describe("sign", () => {
    it("should throw error if message is too short for chain hash", async () => {
      const signer = new Eip712Signer(
        mockProvider as any,
        mockSchema,
        testAddress,
      );

      const shortMessage = new Uint8Array([1, 2, 3]); // Less than 32 bytes
      await expect(signer.sign(shortMessage)).rejects.toThrow(
        "Message too short, expected at least 32 bytes for chain hash",
      );
    });
  });
});
