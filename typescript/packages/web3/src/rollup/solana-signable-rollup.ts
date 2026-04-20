import type SovereignClient from "@sovereign-sdk/client";
import { type Signer, isLedgerSolanaSigner } from "@sovereign-sdk/signers";
import type {
  Transaction,
  TransactionV1,
  UnsignedTransaction,
} from "@sovereign-sdk/types";
import { bytesToHex, hexToBytes } from "@sovereign-sdk/utils";
import bs58 from "bs58";
import { Base64 } from "js-base64";
import type { Subscription, SubscriptionToCallbackMap } from "../subscriptions";
import type { DeepPartial } from "../utils";
import type { RollupConfig, TransactionResult } from "./rollup";
import {
  type StandardRollup,
  type StandardRollupContext,
  type StandardRollupSpec,
  createStandardRollup,
  standardTypeBuilder,
} from "./standard-rollup";

export type SolanaOffchainUnsignedTransaction<RuntimeCall> =
  UnsignedTransaction<RuntimeCall> & {
    chain_name: string;
  };

export type SolanaOffchainUnsignedTransactionV1<RuntimeCall> =
  SolanaOffchainUnsignedTransaction<RuntimeCall> & {
    multisig_id: string;
    version: number;
  };

export type SolanaOffchainSimpleMessage = {
  signed_message: Uint8Array;
  chain_hash: Uint8Array;
  pubkey: Uint8Array;
  signature: Uint8Array;
};

export type SolanaOffchainSpecCompliantMessage = {
  signed_message_with_preamble: Uint8Array;
  signature: Uint8Array;
};

export type SolanaOffchainSimpleMultisigMessage = {
  /** Wire format: [0x80][JSON payload]. The 0x80 prefix is a parsing discriminator only. */
  wire_bytes: Uint8Array;
  chain_hash: Uint8Array;
  signatures: Array<{ signature: Uint8Array; pub_key: Uint8Array }>;
  unused_pub_keys: Uint8Array[];
  min_signers: number;
};

export type SolanaOffchainSpecCompliantMultisigMessage = {
  signed_message_with_preamble: Uint8Array;
  signatures: Uint8Array[];
  signer_bitfield: number;
  min_signers: number;
};

export type Authenticator =
  | "standard"
  | "solanaSimple"
  | "solana"
  | "solanaAuto";

export type SolanaMultisigAuthenticator = "solanaSimple" | "solana";

type SolanaMultisigParams = {
  multisigAddress: Uint8Array;
  multisigPubkeys: Uint8Array[];
};

export type SolanaMultisigSubmitParams =
  | {
      authenticator: "standard";
    }
  | ({ authenticator: SolanaMultisigAuthenticator } & SolanaMultisigParams);

export type SolanaMultisigSignParams = SolanaMultisigSubmitParams & {
  signer: Signer;
};

/**
 * Discriminator byte prepended to the wire bytes in the multisig simple format.
 * Used only for borsh-level format routing — not part of the signed content.
 * Matches `MULTISIG_SIMPLE_DISCRIMINATOR` on the Rust side.
 */
const MULTISIG_SIMPLE_DISCRIMINATOR = 0x80;

// Borsh serialization constants
const VEC_LENGTH_PREFIX_SIZE = 4;
const CHAIN_HASH_SIZE = 32;
const PUBKEY_SIZE = 32;
const SIGNATURE_SIZE = 64;
const MULTISIG_PREAMBLE_FIXED_LENGTH = 53;
const MAX_MULTISIG_SIGNERS = 21; // TODO - is there any way we could get it from Rust rather than hard-coding here

// Solana preamble constants
const SIGNING_DOMAIN = new Uint8Array([
  0xff,
  ...new TextEncoder().encode("solana offchain"),
]);
const HEADER_VERSION = 0;
const MESSAGE_FORMAT = 0;

function compareByteArrays(left: Uint8Array, right: Uint8Array): number {
  const minLength = Math.min(left.length, right.length);

  for (let i = 0; i < minLength; i++) {
    if (left[i] !== right[i]) {
      return left[i] - right[i];
    }
  }

  return left.length - right.length;
}

function createSolanaPreamble(
  pubkeys: Uint8Array[],
  chainHash: Uint8Array,
  messageLength: number,
): Uint8Array {
  if (pubkeys.length < 1 || pubkeys.length > MAX_MULTISIG_SIGNERS) {
    throw new Error(
      `Invalid signer count: expected 1-${MAX_MULTISIG_SIGNERS} signers, got ${pubkeys.length}`,
    );
  }

  const preamble = new Uint8Array(
    MULTISIG_PREAMBLE_FIXED_LENGTH + pubkeys.length * PUBKEY_SIZE,
  );
  let offset = 0;

  preamble.set(SIGNING_DOMAIN, offset);
  offset += 16;

  preamble[offset] = HEADER_VERSION;
  offset += 1;

  preamble.set(chainHash, offset);
  offset += 32;

  preamble[offset] = MESSAGE_FORMAT;
  offset += 1;

  preamble[offset] = pubkeys.length;
  offset += 1;

  for (const pubkey of pubkeys) {
    if (pubkey.length !== PUBKEY_SIZE) {
      throw new Error(
        `Invalid public key length: expected ${PUBKEY_SIZE} bytes, got ${pubkey.length}`,
      );
    }

    preamble.set(pubkey, offset);
    offset += PUBKEY_SIZE;
  }

  new DataView(preamble.buffer, offset).setUint16(0, messageLength, true);

  return preamble;
}

export class SolanaSignableRollup<RuntimeCall> {
  private inner: StandardRollup<RuntimeCall>;
  private solanaEndpoint: string;
  private typeBuilder = standardTypeBuilder<StandardRollupSpec<RuntimeCall>>();

  constructor(
    inner: StandardRollup<RuntimeCall>,
    solanaEndpoint = "/sequencer/accept-solana-offchain-tx",
  ) {
    this.inner = inner;
    this.solanaEndpoint = solanaEndpoint;
  }

  /**
   * Determines the appropriate authenticator based on the signer type.
   * Returns "solana" for LedgerSolanaSigner (hardware wallet with spec-compliant signing)
   * and "solanaSimple" for software signers.
   */
  private getAutoAuthenticator(signer: Signer): "solana" | "solanaSimple" {
    return isLedgerSolanaSigner(signer) ? "solana" : "solanaSimple";
  }

  /**
   * Submits serialized data to the Solana endpoint.
   */
  private async submitSerializedMessage(
    serializedMessage: Uint8Array,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    return await this.inner.http.post<SovereignClient.Sequencer.TxCreateResponse>(
      this.solanaEndpoint,
      {
        // Match AcceptTx shape used by standard sequencer endpoints.
        body: { body: Base64.fromUint8Array(serializedMessage) },
      },
    );
  }

  /**
   * Submits a Solana offchain message to the rollup.
   */
  private async submitSolanaMessage(
    solanaMessage: SolanaOffchainSimpleMessage,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    const serializedMessage = this.serializeSolanaMessage(solanaMessage);
    return this.submitSerializedMessage(serializedMessage);
  }

  /**
   * Submits a Solana spec-compliant message to the rollup.
   */
  private async submitSolanaSpecMessage(
    solanaMessage: SolanaOffchainSpecCompliantMessage,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    const serializedMessage = this.serializeSolanaSpecMessage(solanaMessage);
    return this.submitSerializedMessage(serializedMessage);
  }

  /**
   * Submits a Solana spec-compliant multisig message to the rollup.
   */
  private async submitSolanaSpecMultisigMessage(
    solanaMessage: SolanaOffchainSpecCompliantMultisigMessage,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    const serializedMessage =
      this.serializeSolanaSpecMultisigMessage(solanaMessage);
    return this.submitSerializedMessage(serializedMessage);
  }

  /**
   * Helper to build an unsigned transaction using the standard type builder.
   */
  private async buildUnsignedTransaction(
    runtimeCall: RuntimeCall,
    overrides?: DeepPartial<UnsignedTransaction<RuntimeCall>>,
  ): Promise<UnsignedTransaction<RuntimeCall>> {
    return this.typeBuilder.unsignedTransaction({
      runtimeCall,
      overrides: overrides ?? {},
      rollup: this.inner,
    });
  }

  /**
   * Helper to build a transaction result object.
   */
  private async buildTransactionResult(
    response: SovereignClient.Sequencer.TxCreateResponse,
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    pubkey: Uint8Array,
    signature: Uint8Array,
  ): Promise<TransactionResult<Transaction<RuntimeCall>>> {
    const transaction = await this.typeBuilder.transaction({
      unsignedTx,
      sender: pubkey,
      signature,
      rollup: this.inner,
    });
    return { response, transaction };
  }

  /**
   * Concatenates a Solana preamble with the message bytes it signs.
   */
  private combinePreambleAndMessage(
    preamble: Uint8Array,
    message: Uint8Array,
  ): Uint8Array {
    const signedMessage = new Uint8Array(preamble.length + message.length);
    signedMessage.set(preamble, 0);
    signedMessage.set(message, preamble.length);
    return signedMessage;
  }

  /**
   * Validates and canonicalizes the multisig pubkey list.
   * The canonical order is lexicographic by raw pubkey bytes.
   */
  private canonicalizeMultisigPubkeys(
    multisigPubkeys?: Uint8Array[],
  ): Uint8Array[] {
    if (!multisigPubkeys) {
      throw new Error(
        "multisigPubkeys is required for Solana multisig transactions",
      );
    }

    if (
      multisigPubkeys.length < 2 ||
      multisigPubkeys.length > MAX_MULTISIG_SIGNERS
    ) {
      throw new Error(
        `Invalid multisig signer count: expected 2-${MAX_MULTISIG_SIGNERS} signers, got ${multisigPubkeys.length}`,
      );
    }

    const seenPubkeys = new Set<string>();
    for (const pubkey of multisigPubkeys) {
      if (pubkey.length !== PUBKEY_SIZE) {
        throw new Error(
          `Invalid public key length: expected ${PUBKEY_SIZE} bytes, got ${pubkey.length}`,
        );
      }

      const pubkeyHex = bytesToHex(pubkey);
      if (seenPubkeys.has(pubkeyHex)) {
        throw new Error(`Duplicate multisig public key provided: ${pubkeyHex}`);
      }
      seenPubkeys.add(pubkeyHex);
    }

    return [...multisigPubkeys].sort(compareByteArrays);
  }

  /**
   * Helper to create and serialize a SolanaOffchainUnsignedTransaction to JSON bytes.
   */
  private async createSolanaJsonBytes(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
  ): Promise<Uint8Array> {
    const serializer = await this.inner.serializer();
    const schema = serializer.schema;
    const chainName = schema.chain_data.chain_name || "";

    const solanaUnsignedTx: SolanaOffchainUnsignedTransaction<RuntimeCall> = {
      runtime_call: unsignedTx.runtime_call,
      uniqueness: unsignedTx.uniqueness,
      details: unsignedTx.details,
      chain_name: chainName,
    };

    // JSON serialize the Solana unsigned transaction
    return new TextEncoder().encode(JSON.stringify(solanaUnsignedTx));
  }

  /**
   * Signs an unsigned transaction using Solana offchain simple signing and submits it.
   * Returns the transaction result in the same format as standard rollup.
   */
  private async signWithSolanaSimpleAndSubmit(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    signer: Signer,
  ): Promise<TransactionResult<Transaction<RuntimeCall>>> {
    const jsonBytes = await this.createSolanaJsonBytes(unsignedTx);

    const pubkey = await signer.publicKey();
    const chainHash = await this.inner.chainHash();

    const signature = await signer.sign(jsonBytes);

    // Build and submit result
    const solanaMessage: SolanaOffchainSimpleMessage = {
      signed_message: jsonBytes,
      chain_hash: chainHash,
      pubkey: pubkey,
      signature: signature,
    };

    const response = await this.submitSolanaMessage(solanaMessage);
    return this.buildTransactionResult(response, unsignedTx, pubkey, signature);
  }

  /**
   * Signs an unsigned transaction using Solana spec-compliant signing and submits it.
   * Returns the transaction result in the same format as standard rollup.
   */
  private async signWithSolanaSpecAndSubmit(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    signer: Signer,
  ): Promise<TransactionResult<Transaction<RuntimeCall>>> {
    const jsonBytes = await this.createSolanaJsonBytes(unsignedTx);

    const pubkey = await signer.publicKey();
    const chainHash = await this.inner.chainHash();

    // Create preamble and combine with message
    const preamble = createSolanaPreamble(
      [pubkey],
      chainHash,
      jsonBytes.length,
    );
    const signedMessageWithPreamble = this.combinePreambleAndMessage(
      preamble,
      jsonBytes,
    );
    const signature = await signer.sign(signedMessageWithPreamble);

    // Build and submit result
    const solanaMessage: SolanaOffchainSpecCompliantMessage = {
      signed_message_with_preamble: signedMessageWithPreamble,
      signature: signature,
    };

    const response = await this.submitSolanaSpecMessage(solanaMessage);
    return this.buildTransactionResult(response, unsignedTx, pubkey, signature);
  }

  /**
   * Performs a runtime call transaction with the specified authenticator.
   *
   * @param runtimeCall - The runtime call to execute
   * @param params - Parameters including signer, authenticator type, and optional overrides
   * @param options - Optional request options
   * @returns The transaction result including hash and transaction object
   */
  async call(
    runtimeCall: RuntimeCall,
    params: {
      signer: Signer;
      authenticator: Authenticator;
      overrides?: DeepPartial<UnsignedTransaction<RuntimeCall>>;
    },
    options?: SovereignClient.RequestOptions,
  ): Promise<TransactionResult<Transaction<RuntimeCall>>> {
    const authenticator =
      params.authenticator === "solanaAuto"
        ? this.getAutoAuthenticator(params.signer)
        : params.authenticator;

    // Dispatch based on authenticator
    switch (authenticator) {
      case "standard":
        return this.inner.call(
          runtimeCall,
          {
            signer: params.signer,
            overrides: params.overrides,
          },
          options,
        );
      case "solanaSimple": {
        const unsignedTx = await this.buildUnsignedTransaction(
          runtimeCall,
          params.overrides,
        );
        return this.signWithSolanaSimpleAndSubmit(unsignedTx, params.signer);
      }
      case "solana": {
        const unsignedTx = await this.buildUnsignedTransaction(
          runtimeCall,
          params.overrides,
        );
        return this.signWithSolanaSpecAndSubmit(unsignedTx, params.signer);
      }
      default:
        throw new Error(`Unsupported authenticator: ${authenticator}`);
    }
  }

  /**
   * Signs and submits a transaction with the specified authenticator.
   *
   * @param unsignedTx - The unsigned transaction to sign and submit
   * @param params - Parameters including signer and authenticator type
   * @param options - Optional request options
   * @returns The transaction result
   */
  async signAndSubmitTransaction(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    params: { signer: Signer; authenticator: Authenticator },
    options?: SovereignClient.RequestOptions,
  ): Promise<TransactionResult<Transaction<RuntimeCall>>> {
    const authenticator =
      params.authenticator === "solanaAuto"
        ? this.getAutoAuthenticator(params.signer)
        : params.authenticator;

    switch (authenticator) {
      case "standard":
        return this.inner.signAndSubmitTransaction(
          unsignedTx,
          {
            signer: params.signer,
          },
          options,
        );
      case "solanaSimple":
        return this.signWithSolanaSimpleAndSubmit(unsignedTx, params.signer);
      case "solana":
        return this.signWithSolanaSpecAndSubmit(unsignedTx, params.signer);
      default:
        throw new Error(`Unsupported authenticator: ${authenticator}`);
    }
  }

  /**
   * Creates V1 JSON bytes for a multisig transaction, including multisig_id and version.
   * The resulting JSON is what each signer signs directly (no discriminator prefix).
   */
  private async createMultisigJsonBytes(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    multisigAddress: Uint8Array,
  ): Promise<Uint8Array> {
    const serializer = await this.inner.serializer();
    const schema = serializer.schema;
    const chainName = schema.chain_data.chain_name || "";

    const solanaUnsignedTx: SolanaOffchainUnsignedTransactionV1<RuntimeCall> = {
      runtime_call: unsignedTx.runtime_call,
      uniqueness: unsignedTx.uniqueness,
      details: unsignedTx.details,
      chain_name: chainName,
      // Hardcoded to base58 encoding, only correct for rollups using Base58Address as their
      // primary address type. Will be replaced with rollup-aware address formatting once the
      // SDK supports flexible address encoding (see #2673).
      multisig_id: bs58.encode(multisigAddress),
      version: 1,
    };

    return new TextEncoder().encode(JSON.stringify(solanaUnsignedTx));
  }

  /**
   * Creates the preamble+JSON bytes signed by every signer in a spec-compliant multisig flow.
   */
  private async createSpecCompliantMultisigSignedMessage(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    multisigAddress: Uint8Array,
    multisigPubkeys: Uint8Array[],
  ): Promise<Uint8Array> {
    const jsonBytes = await this.createMultisigJsonBytes(
      unsignedTx,
      multisigAddress,
    );
    const chainHash = await this.inner.chainHash();
    const preamble = createSolanaPreamble(
      multisigPubkeys,
      chainHash,
      jsonBytes.length,
    );

    return this.combinePreambleAndMessage(preamble, jsonBytes);
  }

  /**
   * Signs an unsigned transaction using Solana offchain simple multisig signing.
   */
  private async signForSolanaSimpleMultisig(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    signer: Signer,
    multisigAddress: Uint8Array,
    multisigPubkeys: Uint8Array[],
  ): Promise<Transaction<RuntimeCall>> {
    const pubkey = await signer.publicKey();
    const signerPubkeyHex = bytesToHex(pubkey);
    const multisigPubkeyHexes = multisigPubkeys.map(bytesToHex);

    if (!multisigPubkeyHexes.includes(signerPubkeyHex)) {
      throw new Error(
        `Signer public key ${signerPubkeyHex} is not present in multisigPubkeys`,
      );
    }

    const jsonBytes = await this.createMultisigJsonBytes(
      unsignedTx,
      multisigAddress,
    );

    const signature = await signer.sign(jsonBytes);

    return this.typeBuilder.transaction({
      unsignedTx,
      sender: pubkey,
      signature,
      rollup: this.inner,
    });
  }

  /**
   * Signs an unsigned transaction using the spec-compliant multisig preamble.
   */
  private async signForSolanaSpecMultisig(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    signer: Signer,
    multisigAddress: Uint8Array,
    multisigPubkeys: Uint8Array[],
  ): Promise<Transaction<RuntimeCall>> {
    const pubkey = await signer.publicKey();
    const signerPubkeyHex = bytesToHex(pubkey);
    const multisigPubkeyHexes = multisigPubkeys.map(bytesToHex);

    if (!multisigPubkeyHexes.includes(signerPubkeyHex)) {
      throw new Error(
        `Signer public key ${signerPubkeyHex} is not present in multisigPubkeys`,
      );
    }

    const signedMessageWithPreamble =
      await this.createSpecCompliantMultisigSignedMessage(
        unsignedTx,
        multisigAddress,
        multisigPubkeys,
      );
    const signature = await signer.sign(signedMessageWithPreamble);

    return this.typeBuilder.transaction({
      unsignedTx,
      sender: pubkey,
      signature,
      rollup: this.inner,
    });
  }

  /**
   * Signs an unsigned transaction for use in a Solana offchain multisig.
   *
   * `authenticator: "standard"` delegates directly to the wrapped standard rollup.
   * `authenticator: "solanaSimple"` signs the V1 JSON payload directly.
   * `authenticator: "solana"` signs the spec-compliant preamble+JSON bytes.
   * Solana multisig authenticators treat `multisigPubkeys` as an unordered set and
   * canonicalize it before signing.
   */
  async signTransactionForMultisig(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    params: SolanaMultisigSignParams,
  ): Promise<Transaction<RuntimeCall>> {
    switch (params.authenticator) {
      case "standard":
        return this.inner.signTransaction(unsignedTx, params.signer);
      case "solanaSimple":
        return this.signForSolanaSimpleMultisig(
          unsignedTx,
          params.signer,
          params.multisigAddress,
          this.canonicalizeMultisigPubkeys(params.multisigPubkeys),
        );
      case "solana":
        return this.signForSolanaSpecMultisig(
          unsignedTx,
          params.signer,
          params.multisigAddress,
          this.canonicalizeMultisigPubkeys(params.multisigPubkeys),
        );
    }
  }

  /**
   * Builds a spec-compliant multisig envelope from a V1 transaction and ordered multisig pubkeys.
   */
  private buildSpecCompliantMultisigEnvelope(
    tx: TransactionV1<RuntimeCall>["V1"],
    signedMessageWithPreamble: Uint8Array,
    multisigPubkeys: Uint8Array[],
  ): SolanaOffchainSpecCompliantMultisigMessage {
    const multisigPubkeyHexes = multisigPubkeys.map(bytesToHex);
    const indexByPubkey = new Map<string, number>();

    multisigPubkeyHexes.forEach((pubkeyHex, index) => {
      indexByPubkey.set(pubkeyHex, index);
    });

    const accountedPubkeys = new Set<string>();
    const signaturesByIndex = new Map<number, Uint8Array>();
    let signerBitfield = 0;

    for (const signer of tx.signatures) {
      const signerIndex = indexByPubkey.get(signer.pub_key);
      if (signerIndex === undefined) {
        throw new Error(
          `Signed pubkey ${signer.pub_key} is not present in multisigPubkeys`,
        );
      }
      if (accountedPubkeys.has(signer.pub_key)) {
        throw new Error(
          `Duplicate signed pubkey in multisig transaction: ${signer.pub_key}`,
        );
      }

      accountedPubkeys.add(signer.pub_key);
      signaturesByIndex.set(signerIndex, hexToBytes(signer.signature));
      signerBitfield = (signerBitfield | (1 << signerIndex)) >>> 0;
    }

    for (const unusedPubkey of tx.unused_pub_keys) {
      if (!indexByPubkey.has(unusedPubkey)) {
        throw new Error(
          `Unused pubkey ${unusedPubkey} is not present in multisigPubkeys`,
        );
      }
      if (accountedPubkeys.has(unusedPubkey)) {
        throw new Error(
          `Duplicate pubkey in multisig transaction payload: ${unusedPubkey}`,
        );
      }

      accountedPubkeys.add(unusedPubkey);
    }

    if (accountedPubkeys.size !== multisigPubkeys.length) {
      throw new Error(
        "multisigPubkeys must contain every signer and unused pubkey exactly once",
      );
    }

    const orderedSignatures = Array.from(signaturesByIndex.entries())
      .sort(([leftIndex], [rightIndex]) => leftIndex - rightIndex)
      .map(([, signature]) => signature);

    return {
      signed_message_with_preamble: signedMessageWithPreamble,
      signatures: orderedSignatures,
      signer_bitfield: signerBitfield,
      min_signers: tx.min_signers,
    };
  }

  /**
   * Submits a multisig transaction using the Solana offchain simple multisig format.
   *
   * Accepts the V1 transaction produced by `MultisigTransaction.asTransaction()`.
   * `authenticator: "standard"` delegates directly to the wrapped standard rollup.
   * Solana authenticators rebuild the appropriate Solana multisig envelope and
   * submit it to the Solana offchain endpoint.
   */
  async submitMultisigTransaction(
    multisigTx: TransactionV1<RuntimeCall>,
    params: SolanaMultisigSubmitParams,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    const { V1: tx } = multisigTx;

    const unsignedTx: UnsignedTransaction<RuntimeCall> = {
      runtime_call: tx.runtime_call,
      uniqueness: tx.uniqueness,
      details: tx.details,
    };

    switch (params.authenticator) {
      case "standard":
        return this.inner.submitTransaction(
          multisigTx as StandardRollupSpec<RuntimeCall>["Transaction"],
        );
      case "solanaSimple": {
        this.canonicalizeMultisigPubkeys(params.multisigPubkeys);

        const jsonBytes = await this.createMultisigJsonBytes(
          unsignedTx,
          params.multisigAddress,
        );
        const wireBytes = new Uint8Array(1 + jsonBytes.length);
        wireBytes[0] = MULTISIG_SIMPLE_DISCRIMINATOR;
        wireBytes.set(jsonBytes, 1);

        const chainHash = await this.inner.chainHash();
        const serialized = this.serializeSolanaMultisigMessage({
          wire_bytes: wireBytes,
          chain_hash: chainHash,
          signatures: tx.signatures.map((s) => ({
            signature: hexToBytes(s.signature),
            pub_key: hexToBytes(s.pub_key),
          })),
          unused_pub_keys: tx.unused_pub_keys.map((pk) => hexToBytes(pk)),
          min_signers: tx.min_signers,
        });

        return this.submitSerializedMessage(serialized);
      }
      case "solana": {
        const multisigPubkeys = this.canonicalizeMultisigPubkeys(
          params.multisigPubkeys,
        );
        const signedMessageWithPreamble =
          await this.createSpecCompliantMultisigSignedMessage(
            unsignedTx,
            params.multisigAddress,
            multisigPubkeys,
          );
        const message = this.buildSpecCompliantMultisigEnvelope(
          tx,
          signedMessageWithPreamble,
          multisigPubkeys,
        );

        return this.submitSolanaSpecMultisigMessage(message);
      }
    }
  }

  /**
   * Submits a standard transaction.
   */
  async submitTransaction(
    transaction: StandardRollupSpec<RuntimeCall>["Transaction"],
    authenticator: "standard",
    options?: SovereignClient.RequestOptions,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse>;

  /**
   * Submits a Solana offchain message.
   */
  async submitTransaction(
    transaction: SolanaOffchainSimpleMessage,
    authenticator: "solanaSimple",
    options?: SovereignClient.RequestOptions,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse>;

  /**
   * Submits a Solana spec-compliant message.
   */
  async submitTransaction(
    transaction: SolanaOffchainSpecCompliantMessage,
    authenticator: "solana",
    options?: SovereignClient.RequestOptions,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse>;

  /**
   * Submits a transaction with the specified authenticator.
   *
   * @param transaction - Either a standard transaction or a Solana message
   * @param authenticator - The authenticator type to use
   * @param options - Optional request options
   * @returns The transaction response
   */
  async submitTransaction(
    transaction:
      | StandardRollupSpec<RuntimeCall>["Transaction"]
      | SolanaOffchainSimpleMessage
      | SolanaOffchainSpecCompliantMessage,
    authenticator: Authenticator,
    options?: SovereignClient.RequestOptions,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    switch (authenticator) {
      case "standard":
        return this.inner.submitTransaction(
          transaction as StandardRollupSpec<RuntimeCall>["Transaction"],
          options,
        );
      case "solanaSimple": {
        // For Solana simple, we expect a SolanaOffchainSimpleMessage
        const solanaMessage = transaction as SolanaOffchainSimpleMessage;
        return await this.submitSolanaMessage(solanaMessage);
      }
      case "solana": {
        // For Solana spec-compliant, we expect a SolanaOffchainSpecCompliantMessage
        const solanaMessage = transaction as SolanaOffchainSpecCompliantMessage;
        return await this.submitSolanaSpecMessage(solanaMessage);
      }
      default:
        throw new Error(`Unsupported authenticator: ${authenticator}`);
    }
  }

  async simulate(
    runtimeMessage: RuntimeCall,
    params: Parameters<StandardRollup<RuntimeCall>["simulate"]>[1],
  ) {
    return this.inner.simulate(runtimeMessage, params);
  }

  async dedup(address: Uint8Array) {
    return this.inner.dedup(address);
  }

  async serializer() {
    return this.inner.serializer();
  }

  async chainHash() {
    return this.inner.chainHash();
  }

  /**
   * Pre-fetches the rollup schema, populating the serializer and chain hash caches.
   * Call this during initialization to avoid the latency of lazy-loading on the first transaction.
   */
  hydrate(): Promise<void> {
    return this.inner.hydrate();
  }

  async healthcheck(timeout?: number) {
    return this.inner.healthcheck(timeout);
  }

  subscribe<T extends keyof SubscriptionToCallbackMap>(
    type: T,
    callback: SubscriptionToCallbackMap[T],
  ): Subscription {
    return this.inner.subscribe(type, callback);
  }

  get context() {
    return this.inner.context;
  }

  get ledger() {
    return this.inner.ledger;
  }

  get sequencer() {
    return this.inner.sequencer;
  }

  get rollup() {
    return this.inner.rollup;
  }

  get http() {
    return this.inner.http;
  }

  /**
   * Helper method to serialize a SolanaOffchainSimpleMessage using borsh encoding.
   */
  private serializeSolanaMessage(
    message: SolanaOffchainSimpleMessage,
  ): Uint8Array {
    // Validate message field lengths
    if (message.chain_hash.length !== CHAIN_HASH_SIZE) {
      throw new Error(
        `Invalid chain hash length: expected ${CHAIN_HASH_SIZE} bytes, got ${message.chain_hash.length}`,
      );
    }

    if (message.pubkey.length !== PUBKEY_SIZE) {
      throw new Error(
        `Invalid public key length: expected ${PUBKEY_SIZE} bytes, got ${message.pubkey.length}`,
      );
    }

    if (message.signature.length !== SIGNATURE_SIZE) {
      throw new Error(
        `Invalid signature length: expected ${SIGNATURE_SIZE} bytes, got ${message.signature.length}`,
      );
    }

    // Calculate total size using constants
    const totalSize =
      VEC_LENGTH_PREFIX_SIZE +
      message.signed_message.length +
      CHAIN_HASH_SIZE +
      PUBKEY_SIZE +
      SIGNATURE_SIZE;

    const buffer = new Uint8Array(totalSize);
    let offset = 0;

    // Serialize Vec<u8> with length prefix (little-endian u32)
    const view = new DataView(buffer.buffer);
    view.setUint32(offset, message.signed_message.length, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    buffer.set(message.signed_message, offset);
    offset += message.signed_message.length;

    // Serialize chain_hash [u8; 32]
    buffer.set(message.chain_hash, offset);
    offset += CHAIN_HASH_SIZE;

    // Serialize pubkey [u8; 32]
    buffer.set(message.pubkey, offset);
    offset += PUBKEY_SIZE;

    // Serialize signature [u8; 64]
    buffer.set(message.signature, offset);

    return buffer;
  }

  /**
   * Helper method to serialize a SolanaOffchainSpecCompliantMessage using borsh encoding.
   */
  private serializeSolanaSpecMessage(
    message: SolanaOffchainSpecCompliantMessage,
  ): Uint8Array {
    // Validate signature length
    if (message.signature.length !== SIGNATURE_SIZE) {
      throw new Error(
        `Invalid signature length: expected ${SIGNATURE_SIZE} bytes, got ${message.signature.length}`,
      );
    }

    // Calculate total size
    const totalSize =
      VEC_LENGTH_PREFIX_SIZE +
      message.signed_message_with_preamble.length +
      SIGNATURE_SIZE;

    const buffer = new Uint8Array(totalSize);
    let offset = 0;

    // Serialize Vec<u8> with length prefix (little-endian u32)
    const view = new DataView(buffer.buffer);
    view.setUint32(offset, message.signed_message_with_preamble.length, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    buffer.set(message.signed_message_with_preamble, offset);
    offset += message.signed_message_with_preamble.length;

    // Serialize signature [u8; 64]
    buffer.set(message.signature, offset);

    return buffer;
  }

  /**
   * Serializes a SolanaOffchainSimpleMultisigMessage using borsh encoding.
   * Layout matches the Rust struct:
   *   [u32 LE: wire_bytes.len][wire_bytes]
   *   [32 bytes: chain_hash]
   *   [u32 LE: signatures.len]
   *     for each: [64 bytes: signature][32 bytes: pub_key]
   *   [u32 LE: unused_pub_keys.len]
   *     for each: [32 bytes: pub_key]
   *   [u8: min_signers]
   */
  private serializeSolanaMultisigMessage(
    message: SolanaOffchainSimpleMultisigMessage,
  ): Uint8Array {
    if (message.chain_hash.length !== CHAIN_HASH_SIZE) {
      throw new Error(
        `Invalid chain hash length: expected ${CHAIN_HASH_SIZE} bytes, got ${message.chain_hash.length}`,
      );
    }

    const sigCount = message.signatures.length;
    const unusedCount = message.unused_pub_keys.length;

    const totalSize =
      VEC_LENGTH_PREFIX_SIZE +
      message.wire_bytes.length +
      CHAIN_HASH_SIZE +
      VEC_LENGTH_PREFIX_SIZE +
      sigCount * (SIGNATURE_SIZE + PUBKEY_SIZE) +
      VEC_LENGTH_PREFIX_SIZE +
      unusedCount * PUBKEY_SIZE +
      1; // min_signers u8

    const buffer = new Uint8Array(totalSize);
    const view = new DataView(buffer.buffer);
    let offset = 0;

    // wire_bytes: Vec<u8>
    view.setUint32(offset, message.wire_bytes.length, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    buffer.set(message.wire_bytes, offset);
    offset += message.wire_bytes.length;

    // chain_hash: [u8; 32]
    buffer.set(message.chain_hash, offset);
    offset += CHAIN_HASH_SIZE;

    // signatures: Vec<PubKeyAndSignature>
    view.setUint32(offset, sigCount, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    for (const sig of message.signatures) {
      if (sig.signature.length !== SIGNATURE_SIZE) {
        throw new Error(
          `Invalid signature length: expected ${SIGNATURE_SIZE} bytes, got ${sig.signature.length}`,
        );
      }
      if (sig.pub_key.length !== PUBKEY_SIZE) {
        throw new Error(
          `Invalid public key length: expected ${PUBKEY_SIZE} bytes, got ${sig.pub_key.length}`,
        );
      }
      buffer.set(sig.signature, offset);
      offset += SIGNATURE_SIZE;
      buffer.set(sig.pub_key, offset);
      offset += PUBKEY_SIZE;
    }

    // unused_pub_keys: Vec<PublicKey>
    view.setUint32(offset, unusedCount, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    for (const pk of message.unused_pub_keys) {
      if (pk.length !== PUBKEY_SIZE) {
        throw new Error(
          `Invalid public key length: expected ${PUBKEY_SIZE} bytes, got ${pk.length}`,
        );
      }
      buffer.set(pk, offset);
      offset += PUBKEY_SIZE;
    }

    // min_signers: u8
    buffer[offset] = message.min_signers;

    return buffer;
  }

  /**
   * Serializes a SolanaOffchainSpecCompliantMultisigMessage using borsh encoding.
   * Layout matches the Rust struct:
   *   [u32 LE: signed_message_with_preamble.len][signed_message_with_preamble]
   *   [u32 LE: signatures.len]
   *     for each: [64 bytes: signature]
   *   [u32 LE: signer_bitfield]
   *   [u8: min_signers]
   */
  private serializeSolanaSpecMultisigMessage(
    message: SolanaOffchainSpecCompliantMultisigMessage,
  ): Uint8Array {
    const sigCount = message.signatures.length;

    const totalSize =
      VEC_LENGTH_PREFIX_SIZE +
      message.signed_message_with_preamble.length +
      VEC_LENGTH_PREFIX_SIZE +
      sigCount * SIGNATURE_SIZE +
      4 +
      1;

    const buffer = new Uint8Array(totalSize);
    const view = new DataView(buffer.buffer);
    let offset = 0;

    view.setUint32(offset, message.signed_message_with_preamble.length, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    buffer.set(message.signed_message_with_preamble, offset);
    offset += message.signed_message_with_preamble.length;

    view.setUint32(offset, sigCount, true);
    offset += VEC_LENGTH_PREFIX_SIZE;
    for (const signature of message.signatures) {
      if (signature.length !== SIGNATURE_SIZE) {
        throw new Error(
          `Invalid signature length: expected ${SIGNATURE_SIZE} bytes, got ${signature.length}`,
        );
      }

      buffer.set(signature, offset);
      offset += SIGNATURE_SIZE;
    }

    view.setUint32(offset, message.signer_bitfield >>> 0, true);
    offset += 4;

    buffer[offset] = message.min_signers;

    return buffer;
  }
}

export async function createSolanaSignableRollup<RuntimeCall>(
  rollupConfig?: Partial<RollupConfig<DeepPartial<StandardRollupContext>>>,
  solanaEndpoint = "/sequencer/accept-solana-offchain-tx",
) {
  // Create a standard rollup first
  const standardRollup = await createStandardRollup<RuntimeCall>(rollupConfig);

  // Wrap it with SolanaSignableRollup
  return new SolanaSignableRollup<RuntimeCall>(standardRollup, solanaEndpoint);
}
