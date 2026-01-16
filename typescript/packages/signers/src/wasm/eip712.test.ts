import { beforeEach, describe, expect, it, vi } from "vitest";
import { privateKeyToAccount, signTypedData } from "viem/accounts";
import { hashTypedData } from "viem";
import * as secp from "@noble/secp256k1";
import { Schema, KnownTypeId } from "@sovereign-sdk/universal-wallet-wasm";
import { hexToBytes } from "@sovereign-sdk/utils";
import demoRollupSchema from "../../../__fixtures__/demo-rollup-schema.json";
import { Eip712Signer } from "./eip712";

// Test private key (do not use in production)
const TEST_PRIVATE_KEY =
  "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const testAccount = privateKeyToAccount(TEST_PRIVATE_KEY);
const TEST_PUBLIC_KEY = secp.getPublicKey(hexToBytes(TEST_PRIVATE_KEY.slice(2)), true);

const schema = Schema.fromJSON(JSON.stringify(demoRollupSchema));

// Sample unsigned transaction for testing
const sampleUnsignedTx = {
  runtime_call: {
    bank: {
      transfer: {
        to: { Standard: "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf" },
        coins: {
          amount: "1000",
          token_id: "token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6",
        },
      },
    },
  },
  generation: "12345",
  details: { max_priority_fee_bips: "1000", max_fee: "10000", gas_limit: null, chain_id: "1" },
};

/**
 * Creates the message that would be passed to signer.sign().
 * This is what rollup.signTransaction() does - serialize unsigned tx and append chain hash.
 */
function createTestMessage(): Uint8Array {
  const unsignedTxBorsh = schema.jsonToBorsh(
    schema.knownTypeIndex(KnownTypeId.UnsignedTransaction),
    JSON.stringify(sampleUnsignedTx)
  );
  return new Uint8Array([...unsignedTxBorsh, ...schema.chainHash]);
}

/**
 * Parse typed data received by the mock provider and convert chainId for viem.
 */
function parseTypedDataForViem(typedDataJson: string) {
  const typedData = JSON.parse(typedDataJson);
  const { EIP712Domain: _, ...typesWithoutDomain } = typedData.types;
  const chainId = typeof typedData.domain.chainId === "string"
    ? parseInt(typedData.domain.chainId.replace("0x", ""), 16)
    : typedData.domain.chainId;
  return {
    domain: { ...typedData.domain, chainId },
    types: typesWithoutDomain,
    primaryType: typedData.primaryType as "UnsignedTransaction",
    message: typedData.message,
  };
}

/**
 * Creates a mock provider that signs whatever typed data the signer sends.
 * The provider receives typed data from the signer (not pre-computed in tests).
 */
function createMockProvider() {
  return {
    request: vi.fn(async ({ method, params }: { method: string; params?: unknown[] }) => {
      if (method === "eth_signTypedData_v4") {
        const [, typedDataJson] = params as [string, string];
        const viemData = parseTypedDataForViem(typedDataJson);
        return signTypedData({ privateKey: TEST_PRIVATE_KEY, ...viemData });
      }
      throw new Error(`Unsupported method: ${method}`);
    }),
  };
}

describe("Eip712Signer", () => {
  const mockProvider = { request: vi.fn() };
  const testAddress = testAccount.address;

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("constructor", () => {
    it("should create a signer instance with address", () => {
      const signer = new Eip712Signer(mockProvider as any, demoRollupSchema, testAddress);
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
      const signer = new Eip712Signer(mockProvider as any, demoRollupSchema, testAddress);
      await expect(signer.publicKey()).rejects.toThrow(
        "Public key was not available, you must call sign() first"
      );
    });
  });

  describe("sign", () => {
    it("should throw error if message is too short for chain hash", async () => {
      const signer = new Eip712Signer(mockProvider as any, demoRollupSchema, testAddress);
      const shortMessage = new Uint8Array([1, 2, 3]);
      await expect(signer.sign(shortMessage)).rejects.toThrow(
        "Message too short, expected at least 32 bytes for chain hash"
      );
    });

    it("should sign and return 64-byte signature with correct public key", async () => {
      const message = createTestMessage();
      const provider = createMockProvider();
      const signer = new Eip712Signer(provider as any, demoRollupSchema, testAddress);

      const signature = await signer.sign(message);

      // Should return a 64-byte compact signature
      expect(signature).toBeInstanceOf(Uint8Array);
      expect(signature.length).toBe(64);

      // Public key should be available and match expected
      const publicKey = await signer.publicKey();
      expect(publicKey).toEqual(TEST_PUBLIC_KEY);

      // Verify signature using the typed data the signer sent to the provider
      const [, typedDataJson] = provider.request.mock.calls[0][0].params as [string, string];
      const viemData = parseTypedDataForViem(typedDataJson);
      const signingHash = hexToBytes(hashTypedData(viemData).slice(2));
      expect(secp.verify(signature, signingHash, TEST_PUBLIC_KEY)).toBe(true);
    });

    it("should normalize high-s signatures to low-s", async () => {
      const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
      const halfN = N / 2n;
      const message = createTestMessage();

      // Mock provider that forces HIGH-S signature
      const highSProvider = {
        request: vi.fn(async ({ method, params }: { method: string; params?: unknown[] }) => {
          if (method === "eth_signTypedData_v4") {
            const [, typedDataJson] = params as [string, string];
            const viemData = parseTypedDataForViem(typedDataJson);
            const sig = await signTypedData({ privateKey: TEST_PRIVATE_KEY, ...viemData });

            // Parse and force high-s
            const sigBytes = hexToBytes(sig.slice(2));
            const r = sigBytes.slice(0, 32);
            const s = sigBytes.slice(32, 64);
            const v = sigBytes[64];

            const sBigInt = BigInt("0x" + Array.from(s).map((b) => b.toString(16).padStart(2, "0")).join(""));
            const newS = N - sBigInt; // Force high-s
            const newV = v === 27 ? 28 : (v === 28 ? 27 : v);

            const newSBytes = hexToBytes(newS.toString(16).padStart(64, "0"));
            const highSSig = new Uint8Array(65);
            highSSig.set(r, 0);
            highSSig.set(newSBytes, 32);
            highSSig[64] = newV;

            return "0x" + Array.from(highSSig).map((b) => b.toString(16).padStart(2, "0")).join("");
          }
          throw new Error(`Unsupported method: ${method}`);
        }),
      };

      const signer = new Eip712Signer(highSProvider as any, demoRollupSchema, testAddress);
      const signature = await signer.sign(message);

      // Verify s is now low (s <= n/2)
      const sBytes = signature.slice(32, 64);
      const s = BigInt("0x" + Array.from(sBytes).map((b) => b.toString(16).padStart(2, "0")).join(""));
      expect(s <= halfN).toBe(true);

      // Verify signature is still valid after normalization
      const [, typedDataJson] = highSProvider.request.mock.calls[0][0].params as [string, string];
      const viemData = parseTypedDataForViem(typedDataJson);
      const signingHash = hexToBytes(hashTypedData(viemData).slice(2));
      expect(secp.verify(signature, signingHash, TEST_PUBLIC_KEY)).toBe(true);
    });
  });
});
