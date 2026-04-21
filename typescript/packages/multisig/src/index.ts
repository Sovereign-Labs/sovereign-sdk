import { sha256 } from "@noble/hashes/sha2";
import type {
  SignatureAndPubKey,
  TransactionV1,
  UnsignedTransactionV0,
} from "@sovereign-sdk/types";
import type { HexString } from "@sovereign-sdk/utils";
import { hexToBytes, normalizeHexString } from "@sovereign-sdk/utils";
import * as borsh from "borsh";

const MAX_SIGNERS = 21;

/**
 * Base error class for multisig-related errors.
 */
export class MultisigError extends Error {}

/**
 * Error thrown when multisig is constructed or used with invalid parameters.
 */
export class InvalidMultisigParameterError extends MultisigError {
  constructor(desc: string) {
    super(`Multisig was constructed with invalid parameter: ${desc}`);
  }
}

/**
 * Parameters for constructing a Multisig.
 */
export type MultisigParams = {
  /** Array of signatures with pubkeys already collected */
  signatures?: SignatureAndPubKey[];
  /** Array of public keys that haven't signed yet */
  unusedPubKeys: HexString[];
  /** Minimum number of signatures required */
  minSigners: number;
};

/**
 * Represents multisig signer membership and collected signatures.
 */
export class Multisig {
  private signatures: SignatureAndPubKey[];
  private minSigners: number;
  private unusedPubKeys: Set<HexString>;

  constructor({ signatures = [], unusedPubKeys, minSigners }: MultisigParams) {
    const normalizedSignatures = signatures.map(normalizeSignatureAndPubKey);
    const normalizedUnusedPubKeys = unusedPubKeys.map(normalizeHexString);

    assertValidSignerSet(
      normalizedSignatures,
      normalizedUnusedPubKeys,
      minSigners,
    );

    this.signatures = normalizedSignatures;
    this.unusedPubKeys = new Set(normalizedUnusedPubKeys);
    this.minSigners = minSigners;
  }

  static fromPubKeys(allPubKeys: HexString[], minSigners: number): Multisig {
    return new Multisig({
      signatures: [],
      unusedPubKeys: allPubKeys,
      minSigners,
    });
  }

  addSignature(signature: HexString, pubKey: HexString): void;
  addSignature(pair: SignatureAndPubKey): void;
  addSignature(
    signatureOrPair: HexString | SignatureAndPubKey,
    pubKey?: HexString,
  ): void {
    const pair = normalizeSignatureAndPubKey(
      typeof signatureOrPair === "string"
        ? {
            signature: normalizeHexString(signatureOrPair),
            pub_key: normalizeHexString(assertDefined(pubKey)),
          }
        : signatureOrPair,
    );

    if (!this.unusedPubKeys.delete(pair.pub_key)) {
      throw new InvalidMultisigParameterError(
        `Public key is not a member of the multisig or has already signed: ${pair.pub_key}`,
      );
    }

    this.signatures.push(pair);
  }

  get isComplete(): boolean {
    return this.signatures.length >= this.minSigners;
  }

  get threshold(): number {
    return this.minSigners;
  }

  get signaturesAndPubKeys(): Readonly<SignatureAndPubKey[]> {
    return this.signatures.map((pair) => ({ ...pair }));
  }

  get remainingPubKeys(): Readonly<Set<HexString>> {
    return new Set(this.unusedPubKeys);
  }

  get allPubKeys(): Readonly<HexString[]> {
    return [
      ...this.signatures.map((signature) => signature.pub_key),
      ...this.unusedPubKeys,
    ];
  }

  toTransaction<RuntimeCall>(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
  ): TransactionV1<RuntimeCall> {
    if (!this.isComplete) {
      throw new MultisigError("Multisig transaction is incomplete");
    }

    return {
      V1: {
        ...unsignedTx,
        signatures: [...this.signaturesAndPubKeys],
        unused_pub_keys: [...this.remainingPubKeys],
        min_signers: this.threshold,
      },
    };
  }

  getMultisigAddress(hasher: "sha256" = "sha256"): Uint8Array {
    if (hasher !== "sha256") {
      throw new Error(`Unsupported hasher: ${hasher}`);
    }

    const sortedPubKeys = [...this.allPubKeys].sort();
    const pubKeyBytes = sortedPubKeys.map((pubKey) =>
      Array.from(hexToBytes(pubKey)),
    );
    const buffer = new borsh.BinaryWriter();

    buffer.writeU8(this.minSigners);
    buffer.writeArray(pubKeyBytes, (pkBytes: number[]) => {
      buffer.writeFixedArray(new Uint8Array(pkBytes));
    });

    return sha256(buffer.toArray());
  }
}

function assertValidSignerSet(
  signatures: SignatureAndPubKey[],
  unusedPubKeys: HexString[],
  minSigners: number,
): void {
  const seenPubKeys = new Set<HexString>();
  const allPubKeys = [
    ...signatures.map((signature) => signature.pub_key),
    ...unusedPubKeys,
  ];

  if (!Number.isInteger(minSigners) || minSigners < 1) {
    throw new InvalidMultisigParameterError(
      `minSigners must be a positive integer, got ${minSigners}`,
    );
  }

  if (allPubKeys.length < 2 || allPubKeys.length > MAX_SIGNERS) {
    throw new InvalidMultisigParameterError(
      `expected 2-${MAX_SIGNERS} total signers, got ${allPubKeys.length}`,
    );
  }

  if (minSigners > allPubKeys.length) {
    throw new InvalidMultisigParameterError(
      `minSigners cannot exceed total signer count: ${minSigners} > ${allPubKeys.length}`,
    );
  }

  for (const pubKey of allPubKeys) {
    if (seenPubKeys.has(pubKey)) {
      throw new InvalidMultisigParameterError(
        `Duplicate multisig public key: ${pubKey}`,
      );
    }

    seenPubKeys.add(pubKey);
    hexToBytes(pubKey);
  }
}

function normalizeSignatureAndPubKey(
  pair: SignatureAndPubKey,
): SignatureAndPubKey {
  return {
    signature: normalizeHexString(pair.signature),
    pub_key: normalizeHexString(pair.pub_key),
  };
}

function assertDefined<T>(value: T | undefined): T {
  if (value === undefined) {
    throw new InvalidMultisigParameterError(
      "pubKey is required when adding a signature by positional arguments",
    );
  }

  return value;
}
