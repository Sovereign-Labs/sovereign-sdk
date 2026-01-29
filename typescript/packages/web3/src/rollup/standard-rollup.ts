import SovereignClient from "@sovereign-sdk/client";
import { JsSerializer } from "@sovereign-sdk/serializers";
import type {
  Transaction,
  TxDetails,
  UnsignedTransaction,
} from "@sovereign-sdk/types";
import { bytesToHex } from "@sovereign-sdk/utils";
import type { DeepPartial } from "../utils";
import {
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
};

export type StandardRollupSpec<RuntimeCall> = {
  UnsignedTransaction: UnsignedTransaction<RuntimeCall>;
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
      const { uniqueness: _, ...overrides } = context.overrides;
      const uniqueness = await useOrFetchUniqueness(context);
      const details: TxDetails = {
        ...rollup.context.defaultTxDetails,
        ...overrides.details,
      };

      return {
        runtime_call: runtimeCall,
        uniqueness,
        details,
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
      };
    },
  };
}

/**
 * The parameters for simulating a runtime call transaction.
 */
export type SimulateParams = Omit<
  SovereignClient.RollupSimulateParams,
  "call" | "sender"
> &
  SignerParams;

export class StandardRollup<RuntimeCall> extends Rollup<
  StandardRollupSpec<RuntimeCall>,
  StandardRollupContext
> {
  /**
   * Simulates a runtime call transaction.
   *
   * This method can be useful to estimate the gas cost of a runtime call transaction.
   *
   * @param runtimeMessage - The runtime message to call.
   */
  async simulate(
    runtimeMessage: StandardRollupSpec<RuntimeCall>["RuntimeCall"],
    { signer }: SimulateParams,
  ): Promise<SovereignClient.Rollup.RollupSimulateResponse> {
    const publicKey = await signer.publicKey();
    const sender = bytesToHex(publicKey);
    const call = runtimeMessage as { [key: string]: unknown };

    return this.rollup.simulate({ sender, call });
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
    defaultTxDetails,
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
  const context = await buildContext<C>(client, config.context);

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
