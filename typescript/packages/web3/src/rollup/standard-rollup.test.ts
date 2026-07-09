import SovereignClient from "@sovereign-sdk/client";
import { Multisig } from "@sovereign-sdk/multisig";
import type { RollupSchema, Serializer } from "@sovereign-sdk/serializers";
import { bytesToHex } from "@sovereign-sdk/utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { addressFromPublicKey } from "../addresses";
import {
  StandardRollup,
  createStandardRollup,
  standardTypeBuilder,
} from "./standard-rollup";

describe("standardTypeBuilder", () => {
  const mockRollup = {
    dedup: vi.fn().mockResolvedValue({ nonce: 5 }),
    serializer: {
      serializeRuntimeCall: vi.fn().mockReturnValue(new Uint8Array([1, 2, 3])),
    },
    context: {
      defaultTxDetails: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_hash_fragment: "1",
      },
    },
    rollup: {
      addresses: {
        dedup: vi.fn().mockResolvedValue({ data: { nonce: 5 } }),
      },
    },
  };

  const builder = standardTypeBuilder();

  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe("unsignedTransaction", () => {
    it("should use provided generation from overrides", async () => {
      const result = await builder.unsignedTransaction({
        runtimeCall: { foo: "bar" },
        overrides: { uniqueness: { generation: 10 } },
        rollup: mockRollup as any,
      });

      expect(result).toEqual({
        runtime_call: { foo: "bar" },
        uniqueness: { generation: 10 },
        details: {
          max_priority_fee_bips: 100,
          max_fee: "1000",
          chain_hash_fragment: "1",
        },
        address_override: null,
      });
    });

    it("should use current unix timestamp for generation if not provided in overrides", async () => {
      vi.setSystemTime(1709211600000);

      const result = await builder.unsignedTransaction({
        runtimeCall: { foo: "bar" },
        overrides: {},
        rollup: mockRollup as any,
      });

      expect(result).toEqual({
        runtime_call: { foo: "bar" },
        uniqueness: { generation: 1709211600000 },
        details: {
          max_priority_fee_bips: 100,
          max_fee: "1000",
          chain_hash_fragment: "1",
        },
        address_override: null,
      });
    });

    it("should merge overridden details with defaults", async () => {
      vi.setSystemTime(1709211601100);

      const result = await builder.unsignedTransaction({
        runtimeCall: { foo: "bar" },
        overrides: {
          details: {
            max_fee: "2000",
            gas_limit: [1000000, 1000000],
          },
        },
        rollup: mockRollup as any,
      });

      expect(result).toEqual({
        runtime_call: { foo: "bar" },
        uniqueness: { generation: 1709211601100 },
        details: {
          max_priority_fee_bips: 100,
          max_fee: "2000",
          gas_limit: [1000000, 1000000],
          chain_hash_fragment: "1",
        },
        address_override: null,
      });
    });
  });

  describe("transaction", () => {
    it("should correctly format the transaction", async () => {
      const result = await builder.transaction({
        unsignedTx: {
          runtime_call: {
            value_setter: { set_value: { value: 5, gas: null } },
          },
          uniqueness: { generation: 5 },
          details: {
            max_priority_fee_bips: 100,
            max_fee: "1000",
            chain_hash_fragment: "1",
            gas_limit: null,
          },
          address_override: null,
        },
        sender: new Uint8Array([4, 5, 6]),
        signature: new Uint8Array([7, 8, 9]),
        rollup: mockRollup as any,
      });

      expect(result).toEqual({
        V0: {
          pub_key: "040506",
          signature: "070809",
          runtime_call: {
            value_setter: { set_value: { value: 5, gas: null } },
          },
          uniqueness: { generation: 5 },
          details: {
            max_priority_fee_bips: 100,
            max_fee: "1000",
            chain_hash_fragment: "1",
            gas_limit: null,
          },
          address_override: null,
        },
      });
    });
  });

  describe("transactionSigningPayload", () => {
    it("should format a V0 signing payload with the provided chain hash", async () => {
      const unsignedTx = {
        runtime_call: {
          value_setter: { set_value: { value: 5, gas: null } },
        },
        uniqueness: { generation: 5 },
        details: {
          max_priority_fee_bips: 100,
          max_fee: "1000",
          chain_hash_fragment: "1",
          gas_limit: null,
        },
        address_override: null,
      };

      const result = await builder.transactionSigningPayload({
        unsignedTx,
        chainHash: new Uint8Array([1, 2, 3, 4]),
        rollup: mockRollup as any,
      });

      expect(result).toEqual({
        V0: {
          ...unsignedTx,
          chain_hash: [1, 2, 3, 4],
        },
      });
    });
  });
});

const mockSerializer = {
  serialize: vi.fn().mockReturnValue(new Uint8Array([1, 2, 3])),
  serializeRuntimeCall: vi.fn().mockReturnValue(new Uint8Array([4, 5, 6])),
  serializeSigningPayload: vi.fn().mockReturnValue(new Uint8Array([7, 8, 9])),
  serializeTx: vi.fn().mockReturnValue(new Uint8Array([10, 11, 12])),
  schema: { chainHash: new Uint8Array([1, 2, 3, 4]) } as any,
};

const getSerializer = (_schema: RollupSchema) =>
  mockSerializer as unknown as Serializer;

function createMockStandardClient() {
  const client = new SovereignClient({ fetch: vi.fn() });
  const chainHash =
    "0x0000000000000000000000000000000000000000000000000000000000000000";

  client.rollup = {
    constants: vi.fn().mockResolvedValue({ chain_id: 1 }),
    schema: vi.fn().mockResolvedValue({
      schema: { chain_data: { chain_id: 1, chain_name: "TestChain" } },
      chain_hash: chainHash,
    }),
    addresses: {
      dedup: vi.fn().mockResolvedValue({ nonce: 5 }),
    },
  } as any;
  client.post = vi.fn().mockResolvedValue({ status: "submitted" });

  return client;
}

describe("createStandardRollup", () => {
  const mockConfig = {
    client: new SovereignClient({ fetch: vi.fn() }),
    getSerializer,
    context: {
      defaultTxDetails: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_hash_fragment: "1",
        gas_limit: null,
      },
    },
  };

  it("should create a new client if none is provided", async () => {
    const config = { ...mockConfig, client: undefined };
    const rollup = await createStandardRollup(config);
    expect(rollup.http).toBeInstanceOf(SovereignClient);
  });

  it("should create a new client if none is provided with the specified url", async () => {
    const config = {
      ...mockConfig,
      client: undefined,
      url: "https://example.com",
    };
    const rollup = await createStandardRollup(config);
    expect(rollup.http).toBeInstanceOf(SovereignClient);
    expect(rollup.http.baseURL).toBe("https://example.com");
  });

  it("should create a StandardRollup instance", async () => {
    const rollup = await createStandardRollup(mockConfig);
    expect(rollup).toBeInstanceOf(StandardRollup);
  });

  it("should use the provided type builder overrides", async () => {
    const customUnsignedTransaction = vi.fn();
    const rollup = await createStandardRollup(mockConfig, {
      unsignedTransaction: customUnsignedTransaction,
    });

    // Access the private _typeBuilder
    const typeBuilder = (rollup as any)._typeBuilder;
    expect(typeBuilder.unsignedTransaction).toBe(customUnsignedTransaction);
  });

  it("should maintain default type builder methods when providing partial overrides", async () => {
    const customUnsignedTransaction = vi.fn();
    const rollup = await createStandardRollup(mockConfig, {
      unsignedTransaction: customUnsignedTransaction,
    });

    const typeBuilder = (rollup as any)._typeBuilder;
    expect(typeBuilder.unsignedTransaction).toBe(customUnsignedTransaction);
    expect(typeBuilder.transaction).toBeDefined();
    expect(typeof typeBuilder.transaction).toBe("function");
    expect(typeBuilder.transactionSigningPayload).toBeDefined();
    expect(typeof typeBuilder.transactionSigningPayload).toBe("function");
  });

  it("should be created using the default context", async () => {
    mockConfig.client.rollup.constants = vi
      .fn()
      .mockResolvedValue({ chain_id: 55 });
    mockConfig.client.rollup.schema = vi.fn().mockResolvedValue({
      chain_hash:
        "0x0000000000000000000000000000000000000000000000000000000000000000",
    });
    const rollup = await createStandardRollup({
      ...mockConfig,
      context: undefined,
    });
    expect(rollup.context).toEqual({
      defaultTxDetails: {
        max_priority_fee_bips: 0,
        max_fee: "100000000",
        gas_limit: null,
        chain_hash_fragment: "0",
      },
    });
  });

  it("should preserve supplied context and merge default context", async () => {
    mockConfig.client.rollup.constants = vi
      .fn()
      .mockResolvedValue({ data: { chain_id: 55 } });
    const rollup = await createStandardRollup({
      ...mockConfig,
      context: {
        defaultTxDetails: {
          max_priority_fee_bips: 5,
          chain_hash_fragment: "1",
        },
      },
    });
    expect(rollup.context).toEqual({
      defaultTxDetails: {
        max_priority_fee_bips: 5,
        max_fee: "100000000",
        gas_limit: null,
        chain_hash_fragment: "1",
      },
    });
  });

  it("should pass optional simulation parameters to the client", async () => {
    const client = new SovereignClient({ fetch: vi.fn() });
    client.rollup.simulate = vi.fn().mockResolvedValue({ outcome: "success" });
    const rollup = await createStandardRollup({
      ...mockConfig,
      client,
    });
    const signer = {
      publicKey: vi.fn().mockResolvedValue(new Uint8Array([0xab, 0xcd])),
    };
    const runtimeCall = {
      bank: {
        transfer: {
          to: "receiver",
          coins: { amount: "1", token_id: "token" },
        },
      },
    };
    const txDetails = {
      max_fee: "1234",
    };

    await rollup.simulate(runtimeCall, {
      signer: signer as any,
      address_override: "sov1target",
      tx_details: txDetails,
      uniqueness: { nonce: 7 },
    });

    expect(client.rollup.simulate).toHaveBeenCalledWith({
      sender: "abcd",
      call: runtimeCall,
      address_override: "sov1target",
      tx_details: txDetails,
      uniqueness: { nonce: 7 },
    });
  });

  it("should serialize V0 unsigned transactions as versioned envelopes when signing", async () => {
    const client = createMockStandardClient();
    const serializer = {
      ...mockSerializer,
      serializeSigningPayload: vi
        .fn()
        .mockReturnValue(new Uint8Array([7, 8, 9])),
    };
    const rollup = await createStandardRollup({
      client,
      getSerializer: () => serializer as unknown as Serializer,
      context: mockConfig.context,
    });
    const signer = {
      sign: vi.fn().mockResolvedValue(new Uint8Array([1, 2, 3])),
      publicKey: vi.fn().mockResolvedValue(new Uint8Array([4, 5, 6])),
    };
    const unsignedTx = {
      runtime_call: { test: "call" },
      uniqueness: { nonce: 1 },
      details: mockConfig.context.defaultTxDetails,
      address_override: null,
    };

    await rollup.signTransaction(unsignedTx, signer as any);

    expect(serializer.serializeSigningPayload).toHaveBeenCalledWith({
      V0: {
        ...unsignedTx,
        chain_hash: new Array(32).fill(0),
      },
    });
  });

  it("should fetch dedup data directly by credential id", async () => {
    const client = createMockStandardClient();
    const dedup = vi.fn().mockResolvedValue({ nonce: 9 });
    client.rollup.addresses = { dedup } as any;
    const rollup = await createStandardRollup({
      client,
      getSerializer,
      context: mockConfig.context,
    });

    await expect(rollup.dedupByCredentialId("aabb")).resolves.toEqual({
      nonce: 9,
    });
    expect(dedup).toHaveBeenCalledWith("aabb");
  });

  it("should create multisig signing bytes and finalize via Multisig.toTransaction()", async () => {
    const client = createMockStandardClient();
    const serializer = {
      ...mockSerializer,
      serializeSigningPayload: vi
        .fn()
        .mockReturnValue(new Uint8Array([7, 8, 9])),
    };
    const rollup = await createStandardRollup({
      client,
      getSerializer: () => serializer as unknown as Serializer,
      context: mockConfig.context,
    });
    const signerPublicKey = new Uint8Array(32).fill(7);
    const signer = {
      sign: vi.fn().mockResolvedValue(new Uint8Array(64).fill(8)),
      publicKey: vi.fn().mockResolvedValue(signerPublicKey),
    };
    const otherPublicKey = new Uint8Array(32).fill(9);
    const multisig = Multisig.fromPubKeys(
      [bytesToHex(signerPublicKey), bytesToHex(otherPublicKey)],
      1,
    );
    const unsignedTx = {
      runtime_call: { test: "call" },
      uniqueness: { nonce: 1 },
      details: mockConfig.context.defaultTxDetails,
      address_override: null,
    };

    const signingBytes = await rollup.multisigSigningBytes(
      unsignedTx,
      multisig,
    );

    expect(serializer.serializeSigningPayload).toHaveBeenCalledWith({
      V1: {
        ...unsignedTx,
        chain_hash: new Array(32).fill(0),
        credential_address: addressFromPublicKey(
          multisig.getMultisigAddress(),
          "sov",
        ),
      },
    });
    expect(signingBytes).toEqual(new Uint8Array([7, 8, 9]));

    const signatureBytes = await signer.sign(signingBytes);
    const signature = {
      pub_key: bytesToHex(signerPublicKey),
      signature: bytesToHex(signatureBytes),
    };
    multisig.addSignature(signature.signature, signature.pub_key);

    expect(multisig.toTransaction(unsignedTx)).toEqual({
      V1: {
        ...unsignedTx,
        signatures: [signature],
        unused_pub_keys: [bytesToHex(otherPublicKey)],
        min_signers: 1,
      },
    });
  });

  it("should fail fast when converting an incomplete multisig transaction", () => {
    const multisig = Multisig.fromPubKeys(
      [
        bytesToHex(new Uint8Array(32).fill(1)),
        bytesToHex(new Uint8Array(32).fill(2)),
      ],
      2,
    );
    const unsignedTx = {
      runtime_call: { test: "call" },
      uniqueness: { nonce: 1 },
      details: mockConfig.context.defaultTxDetails,
      address_override: null,
    };

    expect(() => multisig.toTransaction(unsignedTx)).toThrow(
      "Multisig transaction is incomplete",
    );
  });
});
