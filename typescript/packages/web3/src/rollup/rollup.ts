import SovereignClient from "@sovereign-sdk/client";
import type { APIError } from "@sovereign-sdk/client";
import type { RollupSchema, Serializer } from "@sovereign-sdk/serializers";
import type { Signer } from "@sovereign-sdk/signers";
import { bytesToHex, hexToBytes } from "@sovereign-sdk/utils";
import { Base64 } from "js-base64";
import { VersionMismatchError } from "../errors";
import {
  type Subscription,
  type SubscriptionToCallbackMap,
  createSubscription,
} from "../subscriptions";
import type { BaseTypeSpec } from "../type-spec";
import type { DeepPartial } from "../utils";

export type UnsignedTransactionContext<
  S extends BaseTypeSpec,
  C extends RollupContext,
> = {
  runtimeCall: S["RuntimeCall"];
  // Provides the ability to override the generation data instead of retrieving it automatically.
  overrides: DeepPartial<S["UnsignedTransaction"]>;
  rollup: Rollup<S, C>;
};

export type TransactionContext<
  S extends BaseTypeSpec,
  C extends RollupContext,
> = {
  unsignedTx: S["UnsignedTransaction"];
  sender: Uint8Array;
  signature: Uint8Array;
  rollup: Rollup<S, C>;
};

export type TypeBuilder<S extends BaseTypeSpec, C extends RollupContext> = {
  unsignedTransaction: (
    context: UnsignedTransactionContext<S, C>,
  ) => Promise<S["UnsignedTransaction"]>;

  transaction: (context: TransactionContext<S, C>) => Promise<S["Transaction"]>;
};

/**
 * Arbitrary context that is associated with the rollup.
 */
export type RollupContext = Record<string, unknown>;

/**
 * The configuration for a rollup client.
 */
export type RollupConfig<C extends RollupContext> = {
  /**
   * The base URL of the rollup full node API.
   */
  url?: string;
  /**
   * The Sovereign SDK client to use for the rollup.
   * If not provided, the default client will be used using {@link PartialRollupConfig.url}.
   */
  client: SovereignClient;
  /**
   * Creates the serializer instance to use for the rollup.
   *
   * This can be called more than once, for example if the chain hash changes (i.e a new rollup version)
   * is detected then it will be called again with the new rollup schema.
   */
  getSerializer: (schema: RollupSchema) => Serializer;
  /**
   * Arbitrary context that is associated with the rollup.
   */
  context: C;
  /**
   * The endpoint path for submitting transactions.
   * Defaults to "/sequencer/txs", the standard Sovereign SDK endpoint.
   * Can be overridden if your rollup is configured to expose non-standard endpoints in
   * order to support multiple different transaction encoding or authentication schemes.
   */
  txSubmissionEndpoint: string;
};

/**
 * The result of signing and submitting a transaction.
 *
 * @template Tx The type of the transaction
 */
export type TransactionResult<Tx> = {
  /**
   * The response from the sequencer.
   */
  response: SovereignClient.Sequencer.TxCreateResponse;
  /**
   * The transaction that was submitted.
   */
  transaction: Tx;
};

/**
 * The parameters for signing and submitting a transaction.
 */
export type SignerParams = {
  /**
   * The signer to use for signing the transaction.
   */
  signer: Signer;
};

/**
 * The parameters for calling executing a runtime call transaction.
 */
export type CallParams<S extends BaseTypeSpec> = {
  overrides?: DeepPartial<S["UnsignedTransaction"]>;
} & SignerParams;

/**
 * A generic rollup client.
 *
 * @template S - The type specification for the rollup.
 *               If not provided, the base type specification will be used which uses `any` for all types.
 * @template C - The context for the rollup.
 */
export class Rollup<S extends BaseTypeSpec, C extends RollupContext> {
  private readonly _config: RollupConfig<C>;
  private readonly _typeBuilder: TypeBuilder<S, C>;
  private _chainHash?: Uint8Array;
  private _serializer?: Serializer;

  /**
   * Creates a new rollup client.
   *
   * @param config - The configuration for the rollup client.
   * @param typeBuilder - The type builder for the rollup.
   */
  constructor(config: RollupConfig<C>, typeBuilder: TypeBuilder<S, C>) {
    this._config = config;
    this._typeBuilder = typeBuilder;
  }

  /**
   * Retrieve dedup information about the provided address.
   *
   * @param publicKey - The public key to dedup.
   * TODO: How to add param for generation explicitly
   */
  async dedup(publicKey: Uint8Array): Promise<S["Dedup"]> {
    // for public key credential id is just its bytes representation
    const credentialId = bytesToHex(publicKey);
    const response = await this.rollup.addresses.dedup(credentialId);
    return response as S["Dedup"];
  }

  /**
   * Submits a transaction to the rollup.
   *
   * The endpoint used for submission can be overridden via the options.path parameter.
   * If not specified, the rollup's configured txSubmissionEndpoint will be used.
   *
   * @param transaction - The transaction to submit.
   * @param {SovereignClient.RequestOptions} options - The options for the request.
   */
  async submitTransaction(
    transaction: S["Transaction"],
    options?: SovereignClient.RequestOptions,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    const serializer = await this.serializer();
    const serializedTx = serializer.serializeTx(transaction);
    // Stainless RequestOptions is generic internally, causing issues for `body` and `query`.
    // That's fine since we supply the `body` and transaction submission doesn't take query parameters.
    // So we hack around this by destructuring them out.
    const { body: _, query: __, path, ...requestOptions } = options || {};
    const txSubmissionEndpoint = path || this._config.txSubmissionEndpoint;

    return this.http
      .post<SovereignClient.Sequencer.TxCreateResponse>(txSubmissionEndpoint, {
        body: { body: Base64.fromUint8Array(serializedTx) },
        ...requestOptions,
      })
      .catch(async (e) => {
        if (isVersionMismatchError(e as APIError)) {
          const oldVersion = bytesToHex(await this.chainHash());
          const { schema, chain_hash: newVersion } = await this.rollup.schema();
          this._chainHash = hexToBytes(newVersion);
          this._serializer = this._config.getSerializer(schema);

          if (oldVersion !== newVersion) {
            throw new VersionMismatchError(
              "Schema version mismatch when submitting transaction",
              newVersion,
              oldVersion,
            );
          }
        }
        throw e;
      });
  }

  /**
   * Signs and submits a transaction to the rollup.
   * Utilizes the provided signer to sign the transaction.
   *
   * @param unsignedTx - The unsigned transaction to sign and submit.
   * @param {SignerParams} params - The params for signing and submitting the transaction.
   * @param {SovereignClient.RequestOptions} options - The options for the request.
   * @param endpoint - Optional endpoint override. If not specified, uses the rollup's configured txSubmissionEndpoint.
   */
  async signAndSubmitTransaction(
    unsignedTx: S["UnsignedTransaction"],
    { signer }: SignerParams,
    options?: SovereignClient.RequestOptions,
  ): Promise<TransactionResult<S["Transaction"]>> {
    const tx = await this.signTransaction(unsignedTx, signer);
    const result = await this.submitTransaction(tx, options);

    return { transaction: tx, response: result };
  }

  /**
   * Performs a runtime call transaction.
   * Utilizes the provided signer to sign the transaction.
   *
   * @param runtimeCall - The runtime message to call.
   * @param {CallParams<S>} params - The params for submitting a runtime call transaction.
   * @param {SovereignClient.RequestOptions} options - The options for the request.
   */
  async call(
    runtimeCall: S["RuntimeCall"],
    { signer, overrides }: CallParams<S>,
    options?: SovereignClient.RequestOptions,
  ): Promise<TransactionResult<S["Transaction"]>> {
    const context = {
      runtimeCall,
      rollup: this,
      overrides: overrides ?? ({} as DeepPartial<S["UnsignedTransaction"]>),
    };
    const unsignedTx = await this._typeBuilder.unsignedTransaction(context);

    return this.signAndSubmitTransaction(
      unsignedTx,
      {
        signer,
      },
      options,
    );
  }

  /**
   * Prepares a runtime call by creating and signing a transaction without submitting it.
   * This is useful for scenarios where you need the signed transaction for later submission
   * or for analysis purposes.
   *
   * @param runtimeCall - The runtime message to prepare.
   * @param params - The parameters for preparing the call.
   * @param params.signer - The signer to use for signing the transaction.
   * @param params.overrides - Optional overrides for the unsigned transaction.
   * @returns A promise that resolves to the signed transaction.
   */
  async prepareCall(
    runtimeCall: S["RuntimeCall"],
    { signer, overrides }: CallParams<S>,
  ): Promise<S["Transaction"]> {
    const context = {
      runtimeCall,
      rollup: this,
      overrides: overrides ?? ({} as DeepPartial<S["UnsignedTransaction"]>),
    };
    const unsignedTx = await this._typeBuilder.unsignedTransaction(context);
    return this.signTransaction(unsignedTx, signer);
  }

  /**
   * Signs an unsigned transaction using the provided signer.
   * Creates a signature by combining the serialized unsigned transaction with the chain hash,
   * then constructs a fully signed transaction.
   *
   * @param unsignedTx - The unsigned transaction to sign.
   * @param signer - The signer to use for signing the transaction.
   * @returns A promise that resolves to the signed transaction.
   */
  async signTransaction(
    unsignedTx: S["UnsignedTransaction"],
    signer: Signer,
  ): Promise<S["Transaction"]> {
    const serializer = await this.serializer();
    const serializedUnsignedTx = serializer.serializeUnsignedTx(unsignedTx);
    const chainHash = await this.chainHash();
    const signature = await signer.sign(
      new Uint8Array([...serializedUnsignedTx, ...chainHash]),
    );
    const publicKey = await signer.publicKey();
    const context = {
      unsignedTx,
      sender: publicKey,
      signature,
      rollup: this,
    };
    return this._typeBuilder.transaction(context);
  }

  /**
   * Create a subscription to events emitted over websockets by the rollup.
   */
  subscribe<T extends keyof SubscriptionToCallbackMap>(
    type: T,
    callback: SubscriptionToCallbackMap[T],
  ): Subscription {
    return createSubscription(type, callback, this.http);
  }

  /**
   * Performs a healthcheck against the rollup to determine if it is currently healthy.
   *
   * @param timeout - Request timeout in milliseconds.
   * @returns `true` if the rollup is considered healthy otherwise `false`.
   */
  async healthcheck(timeout = 5000): Promise<boolean> {
    try {
      await this.http.get("/healthcheck", { timeout, maxRetries: 1 });
      return true;
    } catch (e) {
      return !(e instanceof SovereignClient.APIConnectionError);
    }
  }

  /**
   * A ledger client that can be used to query the ledger.
   */
  get ledger(): SovereignClient.Ledger {
    return this.http.ledger;
  }

  /**
   * A sequencer client that can be used to submit transactions & batches to the rollup.
   */
  get sequencer(): SovereignClient.Sequencer {
    return this.http.sequencer;
  }

  /**
   * A rollup client that can be used to perform operations on the rollup.
   */
  get rollup(): SovereignClient.Rollup {
    return this.http.rollup;
  }

  /**
   * A client that can be used to perform HTTP requests to the Sovereign API.
   * This can be used as an escape hatch to execute arbitrary requests to the Sovereign API
   * if the operation is not supported by `ledger`, `sequencer`, or `rollup` clients.
   */
  get http(): SovereignClient {
    return this._config.client;
  }

  /**
   * The serializer for the rollup.
   * Can be used to serialize transactions, runtime calls, etc.
   */
  async serializer(): Promise<Serializer> {
    if (this._serializer) return this._serializer;
    const { schema } = await this.rollup.schema();
    this._serializer = this._config.getSerializer(schema);
    return this._serializer;
  }

  /**
   * The context for the rollup.
   */
  get context(): C {
    return this._config.context;
  }

  async chainHash(): Promise<Uint8Array> {
    if (this._chainHash) return this._chainHash;
    const { chain_hash } = await this.rollup.schema();
    this._chainHash = hexToBytes(chain_hash);
    return this._chainHash;
  }
}

function isVersionMismatchError(e: APIError): boolean {
  if (
    // biome-ignore lint/suspicious/noExplicitAny: yolo
    (e.error as any)?.details?.error?.includes("Signature verification failed")
  ) {
    return true;
  }

  return false;
}
