import SovereignClient from "@sovereign-sdk/client";
import type { RollupSchema, Serializer } from "@sovereign-sdk/serializers";
import { beforeEach, describe, expect, it, vi } from "vitest";
import demoRollupSchema from "../../../__fixtures__/demo-rollup-schema.json";
import { VersionMismatchError } from "../errors";
import type { BaseTypeSpec } from "../type-spec";
import {
  Rollup,
  type RollupConfig,
  type RollupContext,
  type TypeBuilder,
} from "./rollup";

const mockSerializer = {
  serialize: vi.fn().mockReturnValue(new Uint8Array([1, 2, 3])),
  serializeRuntimeCall: vi.fn().mockReturnValue(new Uint8Array([4, 5, 6])),
  serializeUnsignedTx: vi.fn().mockReturnValue(new Uint8Array([7, 8, 9])),
  serializeTx: vi.fn().mockReturnValue(new Uint8Array([10, 11, 12])),
  schema: { chainHash: new Uint8Array([1, 2, 3, 4]) } as any,
};

const getSerializer = (_schema: RollupSchema) =>
  mockSerializer as unknown as Serializer;

const testRollup = <S extends BaseTypeSpec, C extends RollupContext>(
  config?: Partial<RollupConfig<C>>,
  builder?: Partial<TypeBuilder<S, C>>,
) => {
  const client = new SovereignClient();
  client.request = vi.fn();
  const rollup = new Rollup(
    {
      client,
      context: {} as C,
      getSerializer,
      txSubmissionEndpoint: "/sequencer/txs",
      ...config,
    },
    {
      unsignedTransaction: vi.fn(),
      transaction: vi.fn(),
      ...builder,
    },
  );

  client.rollup.schema = vi.fn().mockResolvedValue({
    schema: demoRollupSchema,
    chain_hash: "01020304",
  });

  return { rollup, client };
};

describe("Rollup", () => {
  describe("constructor", () => {
    it("should use the provided serializer if it is provided", async () => {
      const { rollup, client } = testRollup({ getSerializer });
      client.rollup.schema = vi.fn().mockResolvedValueOnce({});
      const actual = await rollup.serializer();
      expect(actual).toBe(mockSerializer);
    });
    it("should use the provided client if it is provided", () => {
      const client = new SovereignClient({ fetch: vi.fn() });
      const { rollup } = testRollup({ client });
      expect(rollup.http).toBe(client);
    });
  });
  describe("dedup", () => {
    it("should call the rollup addresses dedup endpoint with hex-encoded address", async () => {
      const { rollup, client } = testRollup();
      client.rollup.addresses.dedup = vi.fn().mockResolvedValue({ nonce: 1 });

      const publicKey = new Uint8Array([1, 2, 3]);
      await rollup.dedup(publicKey);

      expect(client.rollup.addresses.dedup).toHaveBeenCalledWith("010203");
    });

    it("should return the dedup data from the response", async () => {
      const expectedDedup = { nonce: 42 };
      const { rollup, client } = testRollup();
      client.rollup.addresses.dedup = vi.fn().mockResolvedValue(expectedDedup);

      const publicKey = new Uint8Array([1, 2, 3]);
      const result = await rollup.dedup(publicKey);

      expect(result).toEqual(expectedDedup);
    });
  });
  describe("submitTransaction", () => {
    const versionMismatchError = {
      error: {
        details: {
          error: "Signature verification failed",
        },
      },
    };

    it("should correctly serialize and submit the transaction", async () => {
      const { rollup, client } = testRollup();
      client.post = vi.fn().mockResolvedValue({});
      const transaction = { foo: "bar" };

      await rollup.submitTransaction(transaction);

      expect(mockSerializer.serializeTx).toHaveBeenCalledWith(transaction);
      expect(rollup.http.post).toHaveBeenCalledWith("/sequencer/txs", {
        body: {
          body: "CgsM", // Base64 encoded [10,11,12]
        },
      });
    });

    it("should pass options to the sequencer client", async () => {
      const { rollup, client } = testRollup();
      client.post = vi.fn().mockResolvedValue({});
      const transaction = { foo: "bar" };
      const options = { timeout: 5000, maxRetries: 3 };

      await rollup.submitTransaction(transaction, options);

      expect(rollup.http.post).toHaveBeenCalledWith("/sequencer/txs", {
        body: {
          body: "CgsM", // Base64 encoded [10,11,12]
        },
        timeout: 5000,
        maxRetries: 3,
      });
    });

    it("should allow configuring the transaction endpoint", async () => {
      const endpoint = "/sequencer/eip712_tx_for_example";
      const { rollup, client } = testRollup({ txSubmissionEndpoint: endpoint });
      client.post = vi.fn().mockResolvedValue({});
      const transaction = { foo: "bar" };

      await rollup.submitTransaction(transaction);

      expect(rollup.http.post).toHaveBeenCalledWith(
        "/sequencer/eip712_tx_for_example",
        {
          body: {
            body: "CgsM", // Base64 encoded [10,11,12]
          },
        },
      );
    });

    it("should allow overriding the transaction endpoint per-call", async () => {
      const configEndpoint = "/sequencer/txs";
      const overrideEndpoint = "/sequencer/eip712_tx";
      const { rollup, client } = testRollup({
        txSubmissionEndpoint: configEndpoint,
      });
      client.post = vi.fn().mockResolvedValue({});
      const transaction = { foo: "bar" };

      await rollup.submitTransaction(transaction, { path: overrideEndpoint });

      expect(rollup.http.post).toHaveBeenCalledWith("/sequencer/eip712_tx", {
        body: {
          body: "CgsM", // Base64 encoded [10,11,12]
        },
      });
    });

    it("should identify version mismatch errors correctly", async () => {
      const nonVersionMismatchError = {
        error: {
          details: {
            error: "Some other error",
          },
        },
      };

      const { rollup, client } = testRollup();
      client.post = vi.fn().mockRejectedValue(nonVersionMismatchError);
      const transaction = { foo: "bar" };

      await expect(rollup.submitTransaction(transaction)).rejects.toEqual(
        nonVersionMismatchError,
      );
    });

    it("should throw VersionMismatchError when chain hash changes", async () => {
      const { rollup, client } = testRollup();

      client.post = vi.fn().mockRejectedValue(versionMismatchError);
      client.rollup.schema = vi.fn().mockResolvedValue({
        schema: demoRollupSchema,
        chain_hash: "0x00",
      });

      vi.spyOn(rollup, "chainHash").mockResolvedValueOnce(
        new Uint8Array([1, 2, 3, 4]),
      );
      const transaction = { foo: "bar" };

      await expect(rollup.submitTransaction(transaction)).rejects.toThrow(
        VersionMismatchError,
      );
    });

    it("should bubble error if chain hash does not change", async () => {
      const { rollup, client } = testRollup();
      client.post = vi.fn().mockRejectedValue(versionMismatchError);

      vi.spyOn(rollup, "chainHash").mockResolvedValueOnce(
        new Uint8Array([1, 2, 3, 4]),
      );
      const transaction = { foo: "bar" };

      await expect(rollup.submitTransaction(transaction)).rejects.toEqual(
        versionMismatchError,
      );
    });

    it("should propagate non-version-mismatch errors", async () => {
      const { rollup, client } = testRollup();
      const error = new Error("Different error");
      client.post = vi.fn().mockRejectedValue(error);
      const transaction = { foo: "bar" };

      await expect(rollup.submitTransaction(transaction)).rejects.toThrow(
        error,
      );
    });
  });
  describe("signAndSubmitTransaction", () => {
    const mockSigner = {
      sign: vi.fn().mockResolvedValue(new Uint8Array([1, 2, 3])),
      publicKey: vi.fn().mockResolvedValue(new Uint8Array([4, 5, 6])),
    };

    const mockTransaction = { type: "mock-tx" };
    const mockTypeBuilder = {
      transaction: vi.fn().mockResolvedValue(mockTransaction),
    };

    const unsignedTx = { foo: "bar" };

    beforeEach(() => {
      vi.clearAllMocks();
    });

    it("should call signer to sign the unsigned transaction", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      rollup.submitTransaction = vi.fn();

      await rollup.signAndSubmitTransaction(unsignedTx, { signer: mockSigner });

      // should be called with (serialized tx ++ chain hash)
      expect(mockSigner.sign).toHaveBeenCalledWith(
        new Uint8Array([7, 8, 9, 1, 2, 3, 4]),
      );
      expect(mockSerializer.serializeUnsignedTx).toHaveBeenCalledWith(
        unsignedTx,
      );
    });

    it("should pass options to submitTransaction", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      rollup.submitTransaction = vi.fn();
      const options = { timeout: 5000, maxRetries: 3 };

      await rollup.signAndSubmitTransaction(
        unsignedTx,
        { signer: mockSigner },
        options,
      );

      expect(rollup.submitTransaction).toHaveBeenCalledWith(
        mockTransaction,
        options,
      );
    });

    it("should call type builder with correct parameters", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      rollup.submitTransaction = vi.fn();

      await rollup.signAndSubmitTransaction(unsignedTx, { signer: mockSigner });

      expect(mockTypeBuilder.transaction).toHaveBeenCalledWith({
        unsignedTx,
        sender: new Uint8Array([4, 5, 6]),
        signature: new Uint8Array([1, 2, 3]),
        rollup,
      });
    });

    it("should call submitTransaction() with the result of the type builder", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      rollup.submitTransaction = vi.fn();

      await rollup.signAndSubmitTransaction(unsignedTx, { signer: mockSigner });

      expect(rollup.submitTransaction).toHaveBeenCalledWith(
        mockTransaction,
        undefined,
      );
    });

    it("should return the submitted tx and response", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      rollup.submitTransaction = vi
        .fn()
        .mockResolvedValue({ txHash: "mock-hash" });

      const result = await rollup.signAndSubmitTransaction(unsignedTx, {
        signer: mockSigner,
      });

      expect(result).toEqual({
        transaction: mockTransaction,
        response: { txHash: "mock-hash" },
      });
    });
  });
  describe("call", () => {
    const mockSigner = {
      sign: vi.fn().mockResolvedValue(new Uint8Array([1, 2, 3])),
      publicKey: vi.fn().mockResolvedValue(new Uint8Array([4, 5, 6])),
    };

    const mockUnsignedTx = { type: "unsigned-tx" };
    const mockTransaction = { type: "signed-tx" };
    const mockRuntimeCall = { method: "test", args: [] };
    const mockOverrides = { generation: 1 };

    const mockTypeBuilder = {
      unsignedTransaction: vi.fn().mockResolvedValue(mockUnsignedTx),
      transaction: vi.fn().mockResolvedValue(mockTransaction),
    };

    beforeEach(() => {
      vi.clearAllMocks();
    });

    it("should call type builder with correct parameters", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      rollup.submitTransaction = vi.fn();

      await rollup.call(mockRuntimeCall, {
        signer: mockSigner,
        overrides: mockOverrides,
      });

      expect(mockTypeBuilder.unsignedTransaction).toHaveBeenCalledWith({
        runtimeCall: mockRuntimeCall,
        rollup: rollup,
        overrides: mockOverrides,
      });
    });

    it("should pass options to signAndSubmitTransaction", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      const signAndSubmitSpy = vi.spyOn(rollup, "signAndSubmitTransaction");
      rollup.submitTransaction = vi.fn();
      const options = { timeout: 5000, maxRetries: 3 };

      await rollup.call(
        mockRuntimeCall,
        {
          signer: mockSigner,
          overrides: mockOverrides,
        },
        options,
      );

      expect(signAndSubmitSpy).toHaveBeenCalledWith(
        mockUnsignedTx,
        {
          signer: mockSigner,
        },
        options,
      );
    });

    it("should pass the unsigned transaction to signAndSubmitTransaction", async () => {
      const { rollup } = testRollup({}, mockTypeBuilder);
      const signAndSubmitSpy = vi.spyOn(rollup, "signAndSubmitTransaction");
      rollup.submitTransaction = vi.fn();

      await rollup.call(mockRuntimeCall, {
        signer: mockSigner,
        overrides: mockOverrides,
      });

      expect(signAndSubmitSpy).toHaveBeenCalledWith(
        mockUnsignedTx,
        {
          signer: mockSigner,
        },
        undefined,
      );
    });

    it("should return the result from signAndSubmitTransaction", async () => {
      const { rollup, client } = testRollup({}, mockTypeBuilder);
      client.post = vi.fn().mockResolvedValue({ txHash: "mock-hash" });

      const result = await rollup.call(mockRuntimeCall, {
        signer: mockSigner,
        overrides: mockOverrides,
      });

      expect(result).toEqual({
        transaction: mockTransaction,
        response: { txHash: "mock-hash" },
      });
    });

    it("should pass endpoint through to submitTransaction", async () => {
      const endpoint = "/sequencer/eip712_tx";
      const { rollup, client } = testRollup({}, mockTypeBuilder);
      const submitTransactionSpy = vi.spyOn(rollup, "submitTransaction");
      client.post = vi.fn().mockResolvedValue({ txHash: "mock-hash" });

      await rollup.call(
        mockRuntimeCall,
        {
          signer: mockSigner,
        },
        { path: endpoint },
      );

      expect(submitTransactionSpy).toHaveBeenCalledWith(mockTransaction, {
        path: endpoint,
      });
    });
  });
  describe("getters", () => {
    it("should return the ledger client", () => {
      const { rollup, client } = testRollup();

      expect(rollup.ledger).toBe(client.ledger);
    });

    it("should return the configured context", () => {
      const context = { foo: "bar", baz: 123 };
      const { rollup } = testRollup({ context });

      expect(rollup.context).toBe(context);
    });
  });
  describe("healthcheck", () => {
    it("should return false if http request throws APIConnectionError", async () => {
      const { rollup, client } = testRollup();
      client.get = vi
        .fn()
        .mockRejectedValue(new SovereignClient.APIConnectionError({}));

      const result = await rollup.healthcheck();
      expect(result).toBe(false);
    });

    it("should return true if http request throws error unrelated to connection", async () => {
      const { rollup, client } = testRollup();
      client.get = vi.fn().mockRejectedValue(new Error("Some other error"));

      const result = await rollup.healthcheck();
      expect(result).toBe(true);
    });

    it("should return true if http request completes successfully", async () => {
      const { rollup, client } = testRollup();
      client.get = vi.fn().mockResolvedValue({ status: "ok" });

      const result = await rollup.healthcheck();
      expect(result).toBe(true);
    });

    it("should pass timeout to the get request", async () => {
      const { rollup, client } = testRollup();
      client.get = vi.fn().mockResolvedValue({ status: "ok" });

      await rollup.healthcheck(1000);
      expect(client.get).toHaveBeenCalledWith("/healthcheck", {
        timeout: 1000,
        maxRetries: 1,
      });
    });
  });
});
