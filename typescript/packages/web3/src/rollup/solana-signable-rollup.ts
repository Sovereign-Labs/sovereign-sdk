import type SovereignClient from "@sovereign-sdk/client";
import type { Signer } from "@sovereign-sdk/signers";
import { Base64 } from "js-base64";
import type { Subscription, SubscriptionToCallbackMap } from "../subscriptions";
import type { DeepPartial } from "../utils";
import type { RollupConfig, TransactionResult } from "./rollup";
import {
  type StandardRollup,
  type StandardRollupContext,
  type StandardRollupSpec,
  type Transaction,
  type UnsignedTransaction,
  createStandardRollup,
  standardTypeBuilder,
} from "./standard-rollup";

export type SolanaOffchainUnsignedTransaction<RuntimeCall> = {
  runtime_call: RuntimeCall;
  uniqueness: { nonce: number } | { generation: number };
  details: {
    max_priority_fee_bips: number;
    max_fee: string;
    gas_limit: number[] | null;
    chain_id: number;
  };
  chain_name: string;
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

export type Authenticator = "standard" | "solanaSimple" | "solana";

// Borsh serialization constants
const VEC_LENGTH_PREFIX_SIZE = 4;
const CHAIN_HASH_SIZE = 32;
const PUBKEY_SIZE = 32;
const SIGNATURE_SIZE = 64;
const PREAMBLE_LENGTH = 85;

// Solana preamble constants
const SIGNING_DOMAIN = new Uint8Array([
  0xff,
  ...new TextEncoder().encode("solana offchain"),
]);
const HEADER_VERSION = 0;
const MESSAGE_FORMAT = 0;
const SINGLE_SIGNER_COUNT = 1;

/**
 * Creates a Solana offchain message preamble according to the spec.
 * See https://docs.anza.xyz/proposals/off-chain-message-signing#message-preamble
 */
export function createSolanaPreamble(
  pubkey: Uint8Array,
  chainHash: Uint8Array,
  messageLength: number,
): Uint8Array {
  const preamble = new Uint8Array(PREAMBLE_LENGTH);
  let offset = 0;

  // signing_domain: [u8; 16] = b"\xffsolana offchain"
  preamble.set(SIGNING_DOMAIN, offset);
  offset += 16;

  // header_version: u8 = 0 (ASCII format, hw-wallet compatible)
  preamble[offset] = HEADER_VERSION;
  offset += 1;

  // application_domain: [u8; 32] (chain_hash)
  preamble.set(chainHash, offset);
  offset += 32;

  // message_format: u8 = 0 (ASCII format)
  preamble[offset] = MESSAGE_FORMAT;
  offset += 1;

  // signer_count: u8 = 1 (single signer)
  preamble[offset] = SINGLE_SIGNER_COUNT;
  offset += 1;

  // signer: [u8; 32] (public key)
  preamble.set(pubkey, offset);
  offset += 32;

  // message_length: [u8; 2] (little-endian u16)
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
   * Submits serialized data to the Solana endpoint.
   */
  private async submitSerializedMessage(
    serializedMessage: Uint8Array,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    return await this.inner.http.post<
      string,
      SovereignClient.Sequencer.TxCreateResponse
    >(this.solanaEndpoint, {
      body: Base64.fromUint8Array(serializedMessage),
    });
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
   * Helper to create and serialize a SolanaOffchainUnsignedTransaction to JSON bytes.
   */
  private async createSolanaJsonBytes(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
  ): Promise<Uint8Array> {
    const serializer = await this.inner.serializer();
    const schema = serializer.schema;
    const chainName = schema.chain_name || "";

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
    const preamble = createSolanaPreamble(pubkey, chainHash, jsonBytes.length);
    const signedMessageWithPreamble = new Uint8Array(
      preamble.length + jsonBytes.length,
    );
    signedMessageWithPreamble.set(preamble, 0);
    signedMessageWithPreamble.set(jsonBytes, preamble.length);
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
    // Dispatch based on authenticator
    switch (params.authenticator) {
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
        throw new Error(`Unsupported authenticator: ${params.authenticator}`);
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
    switch (params.authenticator) {
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
        throw new Error(`Unsupported authenticator: ${params.authenticator}`);
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
