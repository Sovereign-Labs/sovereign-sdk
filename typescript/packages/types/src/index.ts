import type { HexString } from "@sovereign-sdk/utils";

/**
 * Transaction details specifying fees, gas limits, and network information.
 * Used in all transaction variants to control execution parameters.
 */
export type TxDetails = {
  /** Priority fee in basis points (1/10000ths) for transaction ordering */
  max_priority_fee_bips: number;
  /** Maximum fee willing to pay for transaction execution (as string to handle large numbers) */
  max_fee: string;
  /** Optional gas limit as byte array, null for unlimited */
  gas_limit: number[] | null;
  /** Chain identifier for the target rollup network */
  chain_id: number;
};

/** Timestamp-based uniqueness mechanism using milliseconds since epoch */
export type Generation = { generation: number };

/** Sequential counter-based uniqueness mechanism for ordered transactions */
export type Nonce = { nonce: number };

/** Window-based uniqueness mechanism for non-consecutive account nonces */
export type Window = { window: number };

/** Union type for transaction uniqueness mechanisms */
export type Uniqueness = Nonce | Generation | Window;

/**
 * Common unsigned transaction fields shared by all unsigned transaction versions.
 */
type UnsignedTransactionFields<RuntimeCall> = {
  /** The specific runtime call/method being invoked on the rollup */
  runtime_call: RuntimeCall;
  /** Uniqueness mechanism to prevent replay attacks */
  uniqueness: Uniqueness;
  /** Transaction execution details including fees and gas limits */
  details: TxDetails;
  /** Optional address override for execution; null uses default routing */
  address_override: string | null;
};

/**
 * Consumer-facing version 0 unsigned transaction.
 * Used for standard rollup transaction building and single-signature signing.
 */
export type UnsignedTransactionV0<RuntimeCall> =
  UnsignedTransactionFields<RuntimeCall>;

/**
 * Version 1 unsigned transaction.
 * Used internally for multisig signing bytes and includes the credential commitment.
 */
export type UnsignedTransactionV1<
  RuntimeCall,
  CredentialAddress = unknown,
> = UnsignedTransactionFields<RuntimeCall> & {
  /** The multisig credential address in the rollup's native address format */
  credential_address: CredentialAddress;
};

/**
 * Versioned unsigned transaction envelope.
 * This is the exact root object serialized for signing.
 */
export type UnsignedTransaction<RuntimeCall, CredentialAddress = unknown> =
  | { V0: UnsignedTransactionV0<RuntimeCall> }
  | { V1: UnsignedTransactionV1<RuntimeCall, CredentialAddress> };

/**
 * Version 0 transaction format with single signature.
 * Used for standard single-party transactions.
 */
export type TransactionV0<RuntimeCall> = {
  V0: {
    /** Public key of the transaction signer in hex format */
    pub_key: HexString;
    /** Cryptographic signature of the transaction in hex format */
    signature: HexString;
  } & UnsignedTransactionV0<RuntimeCall>;
};

/**
 * Signature and public key pair structure.
 * Used in multisig transactions to represent individual signer contributions.
 */
export type SignatureAndPubKey = {
  /** Cryptographic signature in hex format */
  signature: HexString;
  /** Public key of the signer in hex format */
  pub_key: HexString;
};

/**
 * Version 1 transaction format supporting multisig operations.
 * Allows multiple signers with configurable threshold requirements.
 */
export type TransactionV1<RuntimeCall> = {
  V1: {
    /** Public keys of multisig members who haven't signed yet */
    unused_pub_keys: HexString[];
    /** Array of signatures from multisig members who have signed */
    signatures: SignatureAndPubKey[];
    /** Minimum number of signatures required for transaction validity */
    min_signers: number;
  } & UnsignedTransactionV0<RuntimeCall>;
};

/**
 * Union type representing all possible transaction variants.
 * Supports both single-signature (V0) and multisig (V1) transactions.
 */
export type Transaction<RuntimeCall> =
  | TransactionV0<RuntimeCall>
  | TransactionV1<RuntimeCall>;
