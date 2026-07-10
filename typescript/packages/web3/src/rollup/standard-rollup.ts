import SovereignClient from "@sovereign-sdk/client";
import type { Multisig } from "@sovereign-sdk/multisig";
import { JsSerializer } from "@sovereign-sdk/serializers";
import type {
  Transaction,
  TransactionSigningPayload,
  TxDetails,
  UnsignedTransaction,
} from "@sovereign-sdk/types";
import { bytesToHex, hexToBytes } from "@sovereign-sdk/utils";
import { addressFromPublicKey } from "../addresses";
import type { DeepPartial } from "../utils";
import {
  type CredentialIdToAddress,
  Rollup,
  type RollupConfig,
  type SignerParams,
  type TransactionContext,
  type TransactionSigningPayloadContext,
  type TypeBuilder,
  type UnsignedTransactionContext,
} from "./rollup";

export type Dedup = {
  nonce: number;
};

export type StandardRollupContext = {
  defaultTxDetails: TxDetails;
  credentialIdToAddress?: CredentialIdToAddress;
};

export type StandardRollupSpec<RuntimeCall> = {
  UnsignedTransaction: UnsignedTransaction<RuntimeCall>;
  TransactionSigningPayload: TransactionSigningPayload<RuntimeCall>;
  Transaction: Transaction<RuntimeCall>;
  RuntimeCall: RuntimeCall;
  Dedup: Dedup;
};

export function chainHashFragment(chainHash: Uint8Array): string {
  if (chainHash.length < 8) {
    throw new Error("chain hash must contain at least 8 bytes");
  }

  let fragment = 0n;
  for (let i = 0; i < 8; i++) {
    fragment |= BigInt(chainHash[i] ?? 0) << BigInt(i * 8);
  }
  return fragment.toString();
}

function refreshChainHashFragment<RuntimeCall>(
  unsignedTx: UnsignedTransaction<RuntimeCall>,
  chainHash: Uint8Array,
): UnsignedTransaction<RuntimeCall> {
  unsignedTx.details = {
    ...unsignedTx.details,
    chain_hash_fragment: chainHashFragment(chainHash),
  };

  return unsignedTx;
}

const useOrFetchUniqueness = async <S extends StandardRollupSpec<unknown>>({
  overrides,
}: Omit<
  UnsignedTransactionContext<S, StandardRollupContext>,
  "runtimeCall"
>) => {
  if (overrides?.uniqueness) {
    return overrides.uniqueness;
  }

  return { generation: Date.now() };
};

export function standardTypeBuilder<
  S extends StandardRollupSpec<unknown>,
>(): TypeBuilder<S, StandardRollupContext> {
  return {
    async unsignedTransaction(
      context: UnsignedTransactionContext<S, StandardRollupContext>,
    ) {
      const { rollup, runtimeCall } = context;
      const overrides = context.overrides as DeepPartial<
        UnsignedTransaction<unknown>
      > & { address_override?: string | null };
      const uniqueness = await useOrFetchUniqueness(context);
      const details: TxDetails = {
        ...rollup.context.defaultTxDetails,
        ...overrides.details,
      };

      return {
        runtime_call: runtimeCall,
        uniqueness,
        details,
        address_override: overrides.address_override ?? null,
      } as S["UnsignedTransaction"];
    },
    async transaction({
      sender,
      signature,
      unsignedTx,
    }: TransactionContext<S, StandardRollupContext>) {
      return {
        V0: {
          pub_key: bytesToHex(sender),
          signature: bytesToHex(signature),
          ...unsignedTx,
        },
      } as S["Transaction"];
    },
    async transactionSigningPayload({
      unsignedTx,
      chainHash,
    }: TransactionSigningPayloadContext<S, StandardRollupContext>) {
      const normalizedUnsignedTx = refreshChainHashFragment(
        unsignedTx,
        chainHash,
      );

      return {
        V0: {
          ...normalizedUnsignedTx,
          chain_hash: Array.from(chainHash),
        },
      } as S["TransactionSigningPayload"];
    },
  };
}

/**
 * The parameters for simulating a runtime call transaction.
 *
 * Adds `address_override` until `@sovereign-sdk/client` is republished with it.
 * As of `0.1.0-alpha.39` the generated `RollupSimulateParams` is missing the
 * field even though the Rust type and OpenAPI spec ship it (added in commit
 * `6e7d22a52`). Once a newer client publishes `address_override` natively,
 * drop this `Omit`-extension AND the `as SovereignClient.RollupSimulateParams`
 * cast in `simulate()`.
 */
export type SimulateParams = Omit<
  SovereignClient.RollupSimulateParams,
  "call" | "sender"
> &
  SignerParams & { address_override?: string | null };

export class StandardRollup<RuntimeCall> extends Rollup<
  StandardRollupSpec<RuntimeCall>,
  StandardRollupContext
> {
  private async credentialAddressFromId(
    credentialId: Uint8Array,
  ): Promise<string> {
    if (this.context.credentialIdToAddress) {
      const serializer = await this.serializer();
      return this.context.credentialIdToAddress(
        credentialId,
        serializer.schema,
      );
    }

    return addressFromPublicKey(credentialId, "sov");
  }

  async signTransaction(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    signer: SignerParams["signer"],
  ): Promise<StandardRollupSpec<RuntimeCall>["Transaction"]> {
    const chainHash = await this.chainHash();

    return super.signTransaction(
      refreshChainHashFragment(unsignedTx, chainHash),
      signer,
    );
  }

  async multisigSigningBytes(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    multisig: Multisig,
  ): Promise<Uint8Array> {
    const serializer = await this.serializer();
    const chainHash = await this.chainHash();
    const normalizedUnsignedTx = refreshChainHashFragment(
      unsignedTx,
      chainHash,
    );
    const signingPayload: TransactionSigningPayload<RuntimeCall> = {
      V1: {
        ...normalizedUnsignedTx,
        chain_hash: Array.from(chainHash),
        credential_address: await this.credentialAddressFromId(
          multisig.getMultisigAddress(),
        ),
      },
    };

    return serializer.serializeSigningPayload(signingPayload);
  }

  /**
   * Simulates a runtime call transaction.
   *
   * This method can be useful to estimate the gas cost of a runtime call transaction.
   *
   * @param runtimeMessage - The runtime message to call.
   */
  async simulate(
    runtimeMessage: StandardRollupSpec<RuntimeCall>["RuntimeCall"],
    { signer, ...params }: SimulateParams,
  ): Promise<SovereignClient.Rollup.RollupSimulateResponse> {
    const publicKey = await signer.publicKey();
    const sender = bytesToHex(publicKey);
    const call = runtimeMessage as { [key: string]: unknown };

    // Cast bridges the `address_override` extension; see SimulateParams JSDoc.
    return this.rollup.simulate({
      ...params,
      sender,
      call,
    } as SovereignClient.RollupSimulateParams);
  }
}

export const DEFAULT_TX_DETAILS: Omit<TxDetails, "chain_hash_fragment"> = {
  max_priority_fee_bips: 0,
  max_fee: "100000000",
  gas_limit: null,
};

async function buildContext<C extends StandardRollupContext>(
  client: SovereignClient,
  context?: DeepPartial<C>,
  credentialIdToAddress?: CredentialIdToAddress,
): Promise<C> {
  const defaultTxDetails = {
    ...DEFAULT_TX_DETAILS,
    ...context?.defaultTxDetails,
  };

  if (!defaultTxDetails.chain_hash_fragment) {
    const { chain_hash } = await client.rollup.schema();

    defaultTxDetails.chain_hash_fragment = chainHashFragment(
      hexToBytes(chain_hash),
    );
  }

  return {
    ...context,
    defaultTxDetails,
    credentialIdToAddress:
      credentialIdToAddress ?? context?.credentialIdToAddress,
  } as C;
}

export async function createStandardRollup<
  RuntimeCall,
  C extends StandardRollupContext = StandardRollupContext,
>(
  rollupConfig?: Partial<RollupConfig<DeepPartial<C>>>,
  typeBuilderOverrides?: Partial<
    TypeBuilder<StandardRollupSpec<RuntimeCall>, C>
  >,
) {
  const config = rollupConfig ?? {};
  const client = config.client ?? new SovereignClient({ baseURL: config.url });
  const getSerializer =
    config.getSerializer ?? ((schema) => new JsSerializer(schema));
  const context = await buildContext<C>(
    client,
    config.context,
    config.credentialIdToAddress,
  );

  // Default to the standard transaction submission endpoint
  const txSubmissionEndpoint = config.txSubmissionEndpoint ?? "/sequencer/txs";

  return new StandardRollup<RuntimeCall>(
    {
      ...config,
      client,
      getSerializer,
      context,
      txSubmissionEndpoint,
    },
    {
      ...standardTypeBuilder(),
      ...typeBuilderOverrides,
    },
  );
}
