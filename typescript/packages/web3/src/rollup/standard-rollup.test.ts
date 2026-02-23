import SovereignClient from "@sovereign-sdk/client";
import type { RollupSchema, Serializer } from "@sovereign-sdk/serializers";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  StandardRollup,
  createRollupHeightUniquenessOverride,
  createStandardRollup,
  fetchCurrentRollupHeight,
  heightUniqueness,
  heightUniquenessBuilderOverride,
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
        chain_id: 1,
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
          chain_id: 1,
        },
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
          chain_id: 1,
        },
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
          chain_id: 1,
        },
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
            chain_id: 1,
            gas_limit: null,
          },
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
            chain_id: 1,
            gas_limit: null,
          },
        },
      });
    });
  });
});

describe("height-based uniqueness helpers", () => {
  const mockRollup = {
    context: {
      defaultTxDetails: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_id: 1,
      },
    },
    http: {
      get: vi.fn(),
    },
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("should fetch the current rollup height", async () => {
    mockRollup.http.get.mockResolvedValue({ value: [42, 99] });

    const height = await fetchCurrentRollupHeight(mockRollup as any);

    expect(height).toBe(42);
    expect(mockRollup.http.get).toHaveBeenCalledWith(
      "/modules/chain-state/state/current-heights",
    );
  });

  it("should build a height-based uniqueness object", async () => {
    mockRollup.http.get.mockResolvedValue({ value: [11, 99] });

    const uniqueness = await heightUniqueness(mockRollup as any);

    expect(uniqueness).toEqual({ height: 11 });
  });

  it("should throw when the current-heights response is missing rollup height", async () => {
    mockRollup.http.get.mockResolvedValue({ value: [] });

    await expect(fetchCurrentRollupHeight(mockRollup as any)).rejects.toThrow(
      "missing rollup height",
    );
  });

  it("should create an unsigned tx override that uses rollup height uniqueness", async () => {
    mockRollup.http.get.mockResolvedValue({ value: [123, 999] });
    const override = createRollupHeightUniquenessOverride();

    const result = await override({
      runtimeCall: { foo: "bar" },
      overrides: {},
      rollup: mockRollup as any,
    });

    expect(result).toEqual({
      runtime_call: { foo: "bar" },
      uniqueness: { height: 123 },
      details: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_id: 1,
      },
    });
  });

  it("should preserve explicit uniqueness overrides when using height override helper", async () => {
    const override = createRollupHeightUniquenessOverride();

    const result = await override({
      runtimeCall: { foo: "bar" },
      overrides: { uniqueness: { generation: 5 } },
      rollup: mockRollup as any,
    });

    expect(result).toEqual({
      runtime_call: { foo: "bar" },
      uniqueness: { generation: 5 },
      details: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_id: 1,
      },
    });
    expect(mockRollup.http.get).not.toHaveBeenCalled();
  });
});

const mockSerializer = {
  serialize: vi.fn().mockReturnValue(new Uint8Array([1, 2, 3])),
  serializeRuntimeCall: vi.fn().mockReturnValue(new Uint8Array([4, 5, 6])),
  serializeUnsignedTx: vi.fn().mockReturnValue(new Uint8Array([7, 8, 9])),
  serializeTx: vi.fn().mockReturnValue(new Uint8Array([10, 11, 12])),
  schema: { chainHash: new Uint8Array([1, 2, 3, 4]) } as any,
};

const getSerializer = (_schema: RollupSchema) =>
  mockSerializer as unknown as Serializer;

describe("createStandardRollup", () => {
  const mockConfig = {
    client: new SovereignClient({ fetch: vi.fn() }),
    getSerializer,
    context: {
      defaultTxDetails: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_id: 1,
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
  });

  it("should support the heightUniquenessBuilderOverride helper", async () => {
    const rollup = await createStandardRollup(
      mockConfig,
      heightUniquenessBuilderOverride(),
    );
    rollup.http.get = vi.fn().mockResolvedValue({ value: [88, 999] });

    const typeBuilder = (rollup as any)._typeBuilder;
    const unsignedTx = await typeBuilder.unsignedTransaction({
      runtimeCall: { foo: "bar" },
      overrides: {},
      rollup,
    });

    expect(unsignedTx).toEqual({
      runtime_call: { foo: "bar" },
      uniqueness: { height: 88 },
      details: {
        max_priority_fee_bips: 100,
        max_fee: "1000",
        chain_id: 1,
        gas_limit: null,
      },
    });
  });

  it("should support manual uniqueness composition in one-off call overrides", async () => {
    const rollup = await createStandardRollup(mockConfig);
    const signer = {
      sign: vi.fn().mockResolvedValue(new Uint8Array([7, 8, 9])),
      publicKey: vi.fn().mockResolvedValue(new Uint8Array([4, 5, 6])),
    };

    rollup.http.get = vi.fn().mockResolvedValue({ value: [66, 999] });
    rollup.http.rollup.schema = vi.fn().mockResolvedValue({
      schema: { chain_data: { chain_id: 1 } },
      chain_hash: "0x01020304",
    });

    const tx = await rollup.prepareCall({ foo: "bar" } as any, {
      signer: signer as any,
      overrides: {
        uniqueness: await heightUniqueness(rollup),
        details: {
          max_fee: "2222",
        },
      },
    });

    expect(tx).toEqual(
      expect.objectContaining({
        V0: expect.objectContaining({
          pub_key: "040506",
          signature: "070809",
          runtime_call: { foo: "bar" },
          uniqueness: { height: 66 },
          details: expect.objectContaining({
            max_priority_fee_bips: 100,
            max_fee: "2222",
            chain_id: 1,
          }),
        }),
      }),
    );
  });

  it("should be created using the default context", async () => {
    mockConfig.client.rollup.constants = vi
      .fn()
      .mockResolvedValue({ chain_id: 55 });
    const rollup = await createStandardRollup({
      ...mockConfig,
      context: undefined,
    });
    expect(rollup.context).toEqual({
      defaultTxDetails: {
        max_priority_fee_bips: 0,
        max_fee: "100000000",
        gas_limit: null,
        chain_id: 55,
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
          chain_id: 1,
        },
      },
    });
    expect(rollup.context).toEqual({
      defaultTxDetails: {
        max_priority_fee_bips: 5,
        max_fee: "100000000",
        gas_limit: null,
        chain_id: 1,
      },
    });
  });
});
