import { bytesToHex } from "@sovereign-sdk/utils";
import { describe, expect, it } from "vitest";
import {
  InvalidMultisigParameterError,
  Multisig,
  type MultisigParams,
} from "./index";

const pubkey1 =
  "33dd646d7c43830b52289c4f277d3a5a26b3ab0a10ee05c871cb654bc046e545";
const pubkey2 =
  "8c7788ad88084f12ce1556703b33e673c5e450eec793c1e8e1a55b1140b4dfb8";
const pubkey3 =
  "74fc1e9c39b21173ef47c1cdfb37158764486c8b8ba81d8f29c6dd77800cd57f";

const createParams = (
  overrides?: Partial<MultisigParams>,
): MultisigParams => ({
  signatures: [],
  unusedPubKeys: [pubkey1, pubkey2, pubkey3],
  minSigners: 2,
  ...overrides,
});

describe("Multisig", () => {
  describe("constructor", () => {
    it("should reject duplicate public keys across signatures and unused pubkeys", () => {
      expect(
        () =>
          new Multisig(
            createParams({
              signatures: [{ pub_key: pubkey1, signature: "aa" }],
              unusedPubKeys: [pubkey1, pubkey2],
            }),
          ),
      ).toThrow(InvalidMultisigParameterError);
    });

    it("should reject thresholds larger than the signer set", () => {
      expect(
        () =>
          new Multisig(
            createParams({
              unusedPubKeys: [pubkey1, pubkey2],
              minSigners: 3,
            }),
          ),
      ).toThrow(InvalidMultisigParameterError);
    });

    it("should reject an empty signer set", () => {
      expect(
        () =>
          new Multisig(
            createParams({
              unusedPubKeys: [],
              minSigners: 1,
            }),
          ),
      ).toThrow(InvalidMultisigParameterError);
    });
  });

  describe("fromPubKeys", () => {
    it("should create a multisig with the full signer set unused", () => {
      const multisig = Multisig.fromPubKeys([pubkey1, pubkey2], 2);

      expect(multisig.threshold).toBe(2);
      expect(multisig.signaturesAndPubKeys).toEqual([]);
      expect([...multisig.remainingPubKeys]).toEqual([pubkey1, pubkey2]);
    });
  });

  describe("addSignature", () => {
    it("should add a signature by pair and remove the signer from remaining keys", () => {
      const multisig = new Multisig(createParams());

      multisig.addSignature({ pub_key: pubkey1, signature: "aa" });

      expect(multisig.signaturesAndPubKeys).toEqual([
        { pub_key: pubkey1, signature: "aa" },
      ]);
      expect([...multisig.remainingPubKeys]).toEqual([pubkey2, pubkey3]);
    });

    it("should add a signature by positional arguments", () => {
      const multisig = new Multisig(createParams());

      multisig.addSignature("aa", pubkey1);

      expect(multisig.signaturesAndPubKeys).toEqual([
        { pub_key: pubkey1, signature: "aa" },
      ]);
    });

    it("should reject unknown or duplicate signers", () => {
      const multisig = new Multisig(createParams());

      expect(() => multisig.addSignature("aa", "11")).toThrow(
        InvalidMultisigParameterError,
      );

      multisig.addSignature("aa", pubkey1);

      expect(() => multisig.addSignature("bb", pubkey1)).toThrow(
        InvalidMultisigParameterError,
      );
    });
  });

  describe("completion", () => {
    it("should track whether enough signatures have been collected", () => {
      const multisig = new Multisig(createParams());

      expect(multisig.isComplete).toBe(false);

      multisig.addSignature("aa", pubkey1);
      expect(multisig.isComplete).toBe(false);

      multisig.addSignature("bb", pubkey2);
      expect(multisig.isComplete).toBe(true);
    });
  });

  describe("getMultisigAddress", () => {
    it("should match the Rust test vector", () => {
      const multisig = Multisig.fromPubKeys([pubkey1, pubkey2, pubkey3], 2);

      expect(bytesToHex(multisig.getMultisigAddress())).toBe(
        "814394e81dd2a682efad0fc2082272cde35a172ba7e0e2240b0a0e9d68af23ee",
      );
    });

    it("should be invariant to current signature collection state", () => {
      const multisig = new Multisig(
        createParams({
          signatures: [{ pub_key: pubkey3, signature: "cc" }],
          unusedPubKeys: [pubkey1, pubkey2],
        }),
      );

      expect(bytesToHex(multisig.getMultisigAddress())).toBe(
        "814394e81dd2a682efad0fc2082272cde35a172ba7e0e2240b0a0e9d68af23ee",
      );
    });
  });
});
