import { beforeEach, describe, expect, it, vi } from "vitest";
import { privateKeyToAccount, signTypedData } from "viem/accounts";
import * as secp from "@noble/secp256k1";
import { Schema, KnownTypeId } from "@sovereign-sdk/universal-wallet-wasm";
import { hexToBytes } from "@sovereign-sdk/utils";
import demoRollupSchema from "../../../__fixtures__/demo-rollup-schema.json";
import { Eip712Signer } from "./eip712";

// Test private key (do not use in production)
const TEST_PRIVATE_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const testAccount = privateKeyToAccount(TEST_PRIVATE_KEY);

const schema = Schema.fromJSON(JSON.stringify(demoRollupSchema));

// Create a sample unsigned transaction
const sampleCall = {
  bank: {
    transfer: {
      to: {
        Standard: "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
      },
      coins: {
        amount: "1000",
        token_id:
          "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6",
      },
    },
  },
};

const sampleUnsignedTx = {
  runtime_call: sampleCall,
  generation: "12345",
  details: {
    max_priority_fee_bips: "1000",
    max_fee: "10000",
    gas_limit: null,
    chain_id: "1",
  },
};

describe("Eip712Signer", () => {
  const mockProvider = {
    request: vi.fn(),
  };

  const testAddress = testAccount.address;

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("constructor", () => {
    it("should create a signer instance with address", () => {
      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      expect(signer).toBeDefined();
      expect(signer.publicKey).toBeDefined();
      expect(signer.sign).toBeDefined();
    });

    it("should throw error if address is not provided", () => {
      expect(() => {
        new Eip712Signer(mockProvider as any, demoRollupSchema, "");
      }).toThrow("Address is required");
    });
  });

  describe("publicKey", () => {
    it("should throw error if public key not available before signing", async () => {
      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      await expect(signer.publicKey()).rejects.toThrow(
        "Public key was not available, you must call sign() first"
      );
    });
  });

  describe("sign", () => {
    it("should throw error if message is too short for chain hash", async () => {
      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      const shortMessage = new Uint8Array([1, 2, 3]); // Less than 32 bytes
      await expect(signer.sign(shortMessage)).rejects.toThrow(
        "Message too short, expected at least 32 bytes for chain hash"
      );
    });

    it("should sign a message and return a 64-byte signature", async () => {
      // Convert unsigned transaction to borsh bytes
      const unsignedTxBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        JSON.stringify(sampleUnsignedTx)
      );

      // Get chain hash
      const chainHash = schema.chainHash;

      // Create the message (unsignedTxBorsh + chainHash)
      const message = new Uint8Array([...unsignedTxBorsh, ...chainHash]);

      // Get the EIP-712 typed data that the signer will generate
      const eip712Json = schema.eip712Json(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        unsignedTxBorsh
      );
      const typedData = JSON.parse(eip712Json);

      // Mock the provider to sign with viem
      mockProvider.request.mockImplementation(async ({ method, params }) => {
        if (method === "eth_signTypedData_v4") {
          const { EIP712Domain: _, ...typesWithoutDomain } = typedData.types;
          return signTypedData({
            privateKey: TEST_PRIVATE_KEY,
            domain: {
              ...typedData.domain,
              chainId: typeof typedData.domain.chainId === 'string'
                ? parseInt(typedData.domain.chainId.replace('0x', ''), 16)
                : typedData.domain.chainId,
            },
            types: typesWithoutDomain,
            primaryType: typedData.primaryType,
            message: typedData.message,
          });
        }
        throw new Error(`Unsupported method: ${method}`);
      });

      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      const signature = await signer.sign(message);

      // Should return a 64-byte compact signature
      expect(signature).toBeInstanceOf(Uint8Array);
      expect(signature.length).toBe(64);
    });

    it("should make the public key available after signing", async () => {
      const unsignedTxBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        JSON.stringify(sampleUnsignedTx)
      );
      const chainHash = schema.chainHash;
      const message = new Uint8Array([...unsignedTxBorsh, ...chainHash]);

      const eip712Json = schema.eip712Json(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        unsignedTxBorsh
      );
      const typedData = JSON.parse(eip712Json);

      mockProvider.request.mockImplementation(async ({ method }) => {
        if (method === "eth_signTypedData_v4") {
          const { EIP712Domain: _, ...typesWithoutDomain } = typedData.types;
          return signTypedData({
            privateKey: TEST_PRIVATE_KEY,
            domain: {
              ...typedData.domain,
              chainId: typeof typedData.domain.chainId === 'string'
                ? parseInt(typedData.domain.chainId.replace('0x', ''), 16)
                : typedData.domain.chainId,
            },
            types: typesWithoutDomain,
            primaryType: typedData.primaryType,
            message: typedData.message,
          });
        }
        throw new Error(`Unsupported method: ${method}`);
      });

      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      await signer.sign(message);

      // Public key should now be available
      const publicKey = await signer.publicKey();
      expect(publicKey).toBeInstanceOf(Uint8Array);
      expect(publicKey.length).toBe(33); // Compressed public key
    });

    it("should recover the correct public key from the signature", async () => {
      const unsignedTxBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        JSON.stringify(sampleUnsignedTx)
      );
      const chainHash = schema.chainHash;
      const message = new Uint8Array([...unsignedTxBorsh, ...chainHash]);

      const eip712Json = schema.eip712Json(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        unsignedTxBorsh
      );
      const typedData = JSON.parse(eip712Json);

      mockProvider.request.mockImplementation(async ({ method }) => {
        if (method === "eth_signTypedData_v4") {
          const { EIP712Domain: _, ...typesWithoutDomain } = typedData.types;
          return signTypedData({
            privateKey: TEST_PRIVATE_KEY,
            domain: {
              ...typedData.domain,
              chainId: typeof typedData.domain.chainId === 'string'
                ? parseInt(typedData.domain.chainId.replace('0x', ''), 16)
                : typedData.domain.chainId,
            },
            types: typesWithoutDomain,
            primaryType: typedData.primaryType,
            message: typedData.message,
          });
        }
        throw new Error(`Unsupported method: ${method}`);
      });

      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      await signer.sign(message);
      const publicKey = await signer.publicKey();

      // The expected public key derived from the test private key
      const expectedPubKey = secp.getPublicKey(
        hexToBytes(TEST_PRIVATE_KEY.slice(2)),
        true // compressed
      );

      expect(publicKey).toEqual(expectedPubKey);
    });

    it("should normalize high-s signatures to low-s", async () => {
      // secp256k1 curve order
      const N =
        0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
      const halfN = N / 2n;

      const unsignedTxBorsh = schema.jsonToBorsh(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        JSON.stringify(sampleUnsignedTx)
      );
      const chainHash = schema.chainHash;
      const message = new Uint8Array([...unsignedTxBorsh, ...chainHash]);

      const eip712Json = schema.eip712Json(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        unsignedTxBorsh
      );
      const typedData = JSON.parse(eip712Json);

      // Mock provider that returns a HIGH-S signature using viem
      mockProvider.request.mockImplementation(async ({ method }) => {
        if (method === "eth_signTypedData_v4") {
          const { EIP712Domain: _, ...typesWithoutDomain } = typedData.types;

          // Use viem to sign (returns 65-byte signature with recovery byte)
          const sig = await signTypedData({
            privateKey: TEST_PRIVATE_KEY,
            domain: {
              ...typedData.domain,
              // viem expects chainId as number
              chainId: typeof typedData.domain.chainId === 'string'
                ? parseInt(typedData.domain.chainId.replace('0x', ''), 16)
                : typedData.domain.chainId,
            },
            types: typesWithoutDomain,
            primaryType: typedData.primaryType,
            message: typedData.message,
          });

          // Parse signature bytes (sig is 0x + 65 bytes hex)
          const sigBytes = hexToBytes(sig.slice(2));
          const r = sigBytes.slice(0, 32);
          const s = sigBytes.slice(32, 64);
          const v = sigBytes[64];

          // Convert s to BigInt
          const sBigInt = BigInt(
            "0x" + Array.from(s).map((b) => b.toString(16).padStart(2, "0")).join("")
          );

          // Force convert to high-s (s' = N - s)
          const newS = N - sBigInt;
          // Flip recovery bit when we flip s
          const newV = v === 27 ? 28 : (v === 28 ? 27 : v);

          // Convert newS back to bytes
          const newSHex = newS.toString(16).padStart(64, "0");
          const newSBytes = hexToBytes(newSHex);

          // Reconstruct signature with high-s
          const highSSig = new Uint8Array(65);
          highSSig.set(r, 0);
          highSSig.set(newSBytes, 32);
          highSSig[64] = newV;

          return "0x" + Array.from(highSSig).map((b) => b.toString(16).padStart(2, "0")).join("");
        }
        throw new Error(`Unsupported method: ${method}`);
      });

      const signer = new Eip712Signer(
        mockProvider as any,
        demoRollupSchema,
        testAddress
      );

      const signature = await signer.sign(message);

      // Extract s value from output signature (last 32 bytes)
      const sBytes = signature.slice(32, 64);
      const s = BigInt(
        "0x" + Array.from(sBytes).map((b) => b.toString(16).padStart(2, "0")).join("")
      );

      // Verify s is now low (s <= n/2) - this tests the normalization
      expect(s <= halfN).toBe(true);

      // Verify the normalized signature is still valid
      const signingHash = schema.eip712SigningHash(
        schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
        unsignedTxBorsh
      );
      const expectedPubKey = secp.getPublicKey(
        hexToBytes(TEST_PRIVATE_KEY.slice(2)),
        true
      );
      const isValid = secp.verify(signature, signingHash, expectedPubKey);
      expect(isValid).toBe(true);
    });
  });
});
