import type {
  Transaction,
  TransactionV0,
  TransactionV1,
  UnsignedTransaction,
} from "@sovereign-sdk/types";
import { describe, expect, it } from "vitest";
import {
  InvalidMultisigParameterError,
  InvalidTransactionVersionError,
  MultisigTransaction,
  TransactionMismatchError,
} from "./index";

const createUnsignedTx = (nonce = 1): UnsignedTransaction<string> => ({
  runtime_call: "test_call",
  uniqueness: { nonce },
  details: {
    max_priority_fee_bips: 100,
    max_fee: "1000",
    gas_limit: null,
    chain_id: 1,
  },
});

const createTransactionV0 = (
  unsignedTx: UnsignedTransaction<string>,
  pubKey: string,
  signature: string,
): TransactionV0<string> => ({
  V0: {
    pub_key: pubKey,
    signature,
    ...unsignedTx,
  },
});

describe("MultisigTransaction", () => {
  describe("fromTransactions", () => {
    it("should require nonce based transactions", () => {
      const unsignedTx: UnsignedTransaction<string> = {
        runtime_call: "test_call",
        uniqueness: { generation: 1 },
        details: {
          max_priority_fee_bips: 100,
          max_fee: "1000",
          gas_limit: null,
          chain_id: 1,
        },
      };
      const tx = createTransactionV0(unsignedTx, "pubkey1", "sig1");

      expect(() =>
        MultisigTransaction.fromTransactions({
          txns: [tx],
          minSigners: 1,
          allPubKeys: ["pubkey1"],
        }),
      ).toThrow(InvalidMultisigParameterError);
    });

    it("should require all inputs to be transaction V0", () => {
      const unsignedTx = createUnsignedTx();
      const txV1: Transaction<string> = {
        V1: {
          unused_pub_keys: ["pubkey2"],
          signatures: [{ pub_key: "pubkey1", signature: "sig1" }],
          min_signers: 1,
          ...unsignedTx,
        },
      };

      expect(() =>
        MultisigTransaction.fromTransactions({
          txns: [txV1],
          minSigners: 1,
          allPubKeys: ["pubkey1"],
        }),
      ).toThrow(InvalidTransactionVersionError);
    });

    it("should require all inputs to match the same unsigned transaction", () => {
      const unsignedTx1 = createUnsignedTx(1);
      const unsignedTx2 = createUnsignedTx(2);
      const tx1 = createTransactionV0(unsignedTx1, "pubkey1", "sig1");
      const tx2 = createTransactionV0(unsignedTx2, "pubkey2", "sig2");

      expect(() =>
        MultisigTransaction.fromTransactions({
          txns: [tx1, tx2],
          minSigners: 2,
          allPubKeys: ["pubkey1", "pubkey2"],
        }),
      ).toThrow(TransactionMismatchError);
    });

    it("should return multisig with only unique signatures (no duplicate transactions)", () => {
      const unsignedTx = createUnsignedTx();
      const tx1 = createTransactionV0(unsignedTx, "pubkey1", "sig1");
      const tx2 = createTransactionV0(unsignedTx, "pubkey2", "sig1"); // same signature

      const multisig = MultisigTransaction.fromTransactions({
        txns: [tx1, tx2],
        minSigners: 2,
        allPubKeys: ["pubkey1", "pubkey2"],
      });

      const result = multisig.asTransaction() as TransactionV1<string>;
      expect(result.V1.signatures).toHaveLength(2);
      expect(result.V1.signatures).toEqual([
        { pub_key: "pubkey1", signature: "sig1" },
        { pub_key: "pubkey2", signature: "sig1" },
      ]);
    });

    it("should throw if an input transaction uses a pubkey not in the unused list", () => {
      const unsignedTx = createUnsignedTx();
      const tx = createTransactionV0(unsignedTx, "unknown_pubkey", "sig1");

      expect(() =>
        MultisigTransaction.fromTransactions({
          txns: [tx],
          minSigners: 1,
          allPubKeys: ["pubkey1", "pubkey2"],
        }),
      ).toThrow(InvalidMultisigParameterError);
    });

    it("should throw if same pubkey is used twice", () => {
      const unsignedTx = createUnsignedTx();
      const tx1 = createTransactionV0(unsignedTx, "pubkey1", "sig1");
      const tx2 = createTransactionV0(unsignedTx, "pubkey1", "sig2");

      expect(() =>
        MultisigTransaction.fromTransactions({
          txns: [tx1, tx2],
          minSigners: 2,
          allPubKeys: ["pubkey1", "pubkey2"],
        }),
      ).toThrow(InvalidMultisigParameterError);
    });
  });

  describe("isComplete", () => {
    it("should return true if number of unique signatures is equal to or greater than minSigners", () => {
      const multisig = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [
          { pub_key: "pubkey1", signature: "sig1" },
          { pub_key: "pubkey2", signature: "sig2" },
          { pub_key: "pubkey3", signature: "sig3" },
        ],
        unusedPubKeys: [],
        minSigners: 2,
      });

      expect(multisig.isComplete).toBe(true);

      const multisigExact = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [
          { pub_key: "pubkey1", signature: "sig1" },
          { pub_key: "pubkey2", signature: "sig2" },
        ],
        unusedPubKeys: [],
        minSigners: 2,
      });

      expect(multisigExact.isComplete).toBe(true);
    });

    it("should return false if number of unique signatures is less than minSigners", () => {
      const multisig = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [{ pub_key: "pubkey1", signature: "sig1" }],
        unusedPubKeys: ["pubkey2", "pubkey3"],
        minSigners: 2,
      });

      expect(multisig.isComplete).toBe(false);
    });
  });

  describe("addSignature", () => {
    it("should throw if pubkey is not in unused list", () => {
      const multisig = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [],
        unusedPubKeys: ["pubkey1", "pubkey2"],
        minSigners: 2,
      });

      expect(() => multisig.addSignature("sig1", "unknown_pubkey")).toThrow(
        InvalidMultisigParameterError,
      );
    });

    it("should add signature to the signatures set", () => {
      const multisig = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [],
        unusedPubKeys: ["pubkey1", "pubkey2"],
        minSigners: 2,
      });

      multisig.addSignature("sig1", "pubkey1");

      const result = multisig.asTransaction() as TransactionV1<string>;
      expect(result.V1.signatures).toEqual([
        { pub_key: "pubkey1", signature: "sig1" },
      ]);
      expect(result.V1.signatures).toHaveLength(1);
    });

    it("should remove pubkey from unused list", () => {
      const multisig = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [],
        unusedPubKeys: ["pubkey1", "pubkey2"],
        minSigners: 2,
      });

      multisig.addSignature("sig1", "pubkey1");

      const result = multisig.asTransaction() as TransactionV1<string>;
      expect(result.V1.unused_pub_keys).not.toContain("pubkey1");
      expect(result.V1.unused_pub_keys).toContain("pubkey2");
      expect(result.V1.unused_pub_keys).toHaveLength(1);
    });

    it("should throw if trying to add signature for already used pubkey", () => {
      const multisig = new MultisigTransaction({
        unsignedTx: createUnsignedTx(),
        signatures: [],
        unusedPubKeys: ["pubkey1", "pubkey2"],
        minSigners: 2,
      });

      multisig.addSignature("sig1", "pubkey1");

      expect(() => multisig.addSignature("sig2", "pubkey1")).toThrow(
        InvalidMultisigParameterError,
      );
    });
  });
});
