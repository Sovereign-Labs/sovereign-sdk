import SovereignClient from "@sovereign-sdk/client";
import { Multisig } from "@sovereign-sdk/multisig";
import { JsSerializer } from "@sovereign-sdk/serializers";
import type {
  SignatureAndPubKey,
  Transaction,
  TransactionV1,
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
 * Adds `address_override` until the regenerated `@sovereign-sdk/client` carries it natively;
 * drop this extension and the cast in `simulate` once the client is republished.
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
  ): Promise<unknown> {
    if (this.context.credentialIdToAddress) {
      const serializer = await this.serializer();
      return this.context.credentialIdToAddress(credentialId, serializer.schema);
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

  private async signVersionedUnsignedTx(
    unsignedTx: UnsignedTransaction<RuntimeCall>,
    signer: SignerParams["signer"],
  ): Promise<SignatureAndPubKey> {
    const serializer = await this.serializer();
    const serializedUnsignedTx = serializer.serializeUnsignedTx(unsignedTx);
    const chainHash = await this.chainHash();
    const signature = await signer.sign(
      new Uint8Array([...serializedUnsignedTx, ...chainHash]),
    );
    const publicKey = await signer.publicKey();

    return {
      pub_key: bytesToHex(publicKey),
      signature: bytesToHex(signature),
    };
  }

  async createMultisigSignature(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
    multisig: Multisig,
    { signer }: SignerParams,
  ): Promise<SignatureAndPubKey> {
    const signingUnsignedTx = await this.multisigUnsignedTxForSigning(
      unsignedTx,
      multisig,
    );

    return this.signVersionedUnsignedTx(signingUnsignedTx, signer);
  }

  async signMultisigTransaction(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
    multisig: Multisig,
    params: SignerParams,
  ): Promise<void> {
    const signature = await this.createMultisigSignature(
      unsignedTx,
      multisig,
      params,
    );
    multisig.addSignature(signature);
  }

  finalizeMultisigTransaction(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
    multisig: Multisig,
  ): TransactionV1<RuntimeCall> {
    return {
      V1: {
        ...unsignedTx,
        signatures: [...multisig.signaturesAndPubKeys],
        unused_pub_keys: [...multisig.remainingPubKeys],
        min_signers: multisig.threshold,
      },
    };
  }

  async submitMultisigTransaction(
    unsignedTx: UnsignedTransactionV0<RuntimeCall>,
    multisig: Multisig,
    options?: SovereignClient.RequestOptions,
  ): Promise<SovereignClient.Sequencer.TxCreateResponse> {
    if (!multisig.isComplete) {
      throw new Error("Multisig transaction is incomplete");
    }

    return this.submitTransaction(
      this.finalizeMultisigTransaction(unsignedTx, multisig),
      options,
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
