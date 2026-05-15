import SovereignClient from "@sovereign-sdk/client";
import type { Multisig } from "@sovereign-sdk/multisig";
import { JsSerializer } from "@sovereign-sdk/serializers";
import type {
  Transaction,
  TxDetails,
  UnsignedTransaction,
  UnsignedTransactionV0,
} from "@sovereign-sdk/types";
import { bytesToHex } from "@sovereign-sdk/utils";
import { addressFromPublicKey } from "../addresses";
import type { DeepPartial } from "../utils";
import {
  type CredentialIdToAddress,
  Rollup,
  type RollupConfig,
  type SignerParams,
  type TransactionContext,
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
  UnsignedTransaction: UnsignedTransactionV0<RuntimeCall>;
  Transaction: Transaction<RuntimeCall>;
  RuntimeCall: RuntimeCall;
  Dedup: Dedup;
};

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
        UnsignedTransactionV0<unknown>
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
  protected async unsignedTxForSigning(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
  ): Promise<UnsignedTransaction<RuntimeCall>> {
    return { V0: unsignedTx };
  }

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

  private async multisigUnsignedTxForSigning(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
    multisig: Multisig,
  ): Promise<UnsignedTransaction<RuntimeCall>> {
    return {
      V1: {
        ...unsignedTx,
        credential_address: await this.credentialAddressFromId(
          multisig.getMultisigAddress(),
        ),
      },
    };
  }

  private async signingBytesForUnsignedTx(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
  ): Promise<Uint8Array> {
    const serializer = await this.serializer();
    const serializedUnsignedTx = serializer.serializeUnsignedTx(unsignedTx);
    const chainHash = await this.chainHash();
    return new Uint8Array([...serializedUnsignedTx, ...chainHash]);
  }

  async multisigSigningBytes(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
    multisig: Multisig,
  ): Promise<Uint8Array> {
    return this.signingBytesForUnsignedTx(
      await this.multisigUnsignedTxForSigning(unsignedTx, multisig),
    );
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

export const DEFAULT_TX_DETAILS: Omit<TxDetails, "chain_id"> = {
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

  if (!defaultTxDetails.chain_id) {
    const { chain_id } = await client.rollup.constants();

    defaultTxDetails.chain_id = chain_id;
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
