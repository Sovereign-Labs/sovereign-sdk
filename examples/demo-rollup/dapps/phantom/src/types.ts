// To parse this data:
//
//   import { Convert, RuntimeCall } from "./file";
//
//   const runtimeCall = Convert.toRuntimeCall(json);
//
// These functions will throw an error if the JSON doesn't
// match the expected interface, even if the JSON is valid.

/**
 * This enum is generated from the underlying Runtime, the variants correspond to call
 * messages from the relevant modules
 *
 * Module call message.
 */
export interface RuntimeCall {
    bank?:                CallMessage;
    sequencer_registry?:  CallMessage2;
    operator_incentives?: CallMessage3;
    attester_incentives?: CallMessage4Class | CallMessage4Enum;
    prover_incentives?:   CallMessage5Class | CallMessage5Enum;
    accounts?:            CallMessage6;
    uniqueness?:          null;
    chain_state?:         CallMessage7Class | CallMessage7Enum;
    blob_storage?:        null;
    paymaster?:           CallMessage8;
    evm?:                 CallMessageClass;
    access_pattern?:      AccessPatternMessagesClass | AccessPatternMessagesEnum;
    synthetic_load?:      CallMessage9;
}

/**
 * Writes `size` bytes to the module state for every position between `begin` and `begin +
 * size`
 *
 * Like [`Self::WriteCells`] but writes a custom string.
 *
 * Reads every element of the module state between `begin` and `begin + size`
 *
 * Hashes the string of bytes made by the repeted filler.
 *
 * Hashes the custom input buffer.
 *
 * Stores a signature to verify.
 *
 * Verifies a custom signature, without storing it to state.
 *
 * Stores a string serialized as bytes.
 *
 * Deserializes a custom input buffer into a string without storing it to state.
 *
 * Deletes every element of the module state between `begin` and `begin + size`
 *
 * Activates the pre/end-exec-hook. Adds a variable number of reads/writes for each tx.
 *
 * Updates the admin for the module.
 */
export interface AccessPatternMessagesClass {
    write_cells?:               WriteCells;
    write_custom?:              WriteCustom;
    read_cells?:                ReadCells;
    hash_bytes?:                HashBytes;
    hash_custom?:               HashCustom;
    store_signature?:           StoreSignature;
    verify_custom_signature?:   VerifyCustomSignature;
    store_serialized_string?:   StoreSerializedString;
    deserialize_custom_string?: DeserializeCustomString;
    delete_cells?:              DeleteCells;
    set_hook?:                  SetHook;
    update_admin?:              AccessPatternMessagesUpdateAdmin;
}

export interface DeleteCells {
    /**
     * The first index to delete from
     */
    begin: number;
    /**
     * The number of storage cells to delete
     */
    num_cells: number;
    [property: string]: any;
}

export interface DeserializeCustomString {
    /**
     * The serialized string to deserialize
     */
    input: number[];
    [property: string]: any;
}

export interface HashBytes {
    /**
     * The filler bytes to be repeated over
     */
    filler: number;
    /**
     * The size of the buffer
     */
    size: number;
    [property: string]: any;
}

export interface HashCustom {
    /**
     * The input to hash
     */
    input: number[];
    [property: string]: any;
}

export interface ReadCells {
    /**
     * The first index to read from
     */
    begin: number;
    /**
     * The number of storage cells to read from
     */
    num_cells: number;
    [property: string]: any;
}

export interface SetHook {
    /**
     * The configuration of the post-exec hooks. Set to None to disable
     */
    post?: HooksConfig[] | null;
    /**
     * The configuration of the pre-exec hooks. Set to None to disable
     */
    pre?: HooksConfig[] | null;
    [property: string]: any;
}

/**
 * Specifies what happens inside the pre/end-exec hook.
 *
 * Reads from the storage
 *
 * Writes to the storage
 *
 * Delete from the storage
 */
export interface HooksConfig {
    Read?:   Read;
    Write?:  Write;
    Delete?: Delete;
}

export interface Delete {
    /**
     * The first index to delete
     */
    begin: number;
    /**
     * The number of storage cells to delete
     */
    size: number;
    [property: string]: any;
}

export interface Read {
    /**
     * The first index to read from
     */
    begin: number;
    /**
     * The number of storage cells to read from
     */
    size: number;
    [property: string]: any;
}

export interface Write {
    /**
     * The first index to write to
     */
    begin: number;
    /**
     * The size of the data to write to each storage cell
     */
    data_size: number;
    /**
     * The number of storage cells to write to
     */
    size: number;
    [property: string]: any;
}

export interface StoreSerializedString {
    /**
     * The serialized string to store
     */
    input: number[];
    [property: string]: any;
}

export interface StoreSignature {
    /**
     * The associated message
     */
    message: string;
    /**
     * The associated public key
     */
    pub_key: Ed25519PublicKey;
    /**
     * The signature to store
     */
    sign: Ed25519Signature;
    [property: string]: any;
}

/**
 * The associated public key
 *
 * The public key of an ed25519 keypair.
 */
export interface Ed25519PublicKey {
    pub_key: number[];
    [property: string]: any;
}

/**
 * The signature to store
 *
 * An ed25519 signature. Wraps the optimized Risc0 fork of the ed25519-dalek crate.
 */
export interface Ed25519Signature {
    bytes: number[];
    /**
     * The inner signature.
     */
    msg_sig: number[];
    [property: string]: any;
}

export interface AccessPatternMessagesUpdateAdmin {
    /**
     * New admin of the module
     */
    new_admin: MultiAddressEvmSolana;
    [property: string]: any;
}

/**
 * An address type which supports standard rollup addresses, EVM addresses, and Solana-style
 * base58 addresses.
 *
 * The address of the account that the new tokens are minted to.
 *
 * The address to which the tokens will be transferred.
 *
 * Address to mint tokens to
 *
 * The new address that will receive rewards for operating the rollup. Note: We do not
 * verify possession of the corresponding private key, so it's possible to set an address
 * for which the `sender` does not control the private key.
 *
 * New admin of the module
 *
 * A standard address derived from a SHA-256 hash of a public key.
 *
 * A 20-byte Ethereum address.
 *
 * A 32-byte Solana-style base58 address.
 */
export interface MultiAddressEvmSolana {
    Standard?: string;
    Evm?:      string;
    Solana?:   string;
}

export interface VerifyCustomSignature {
    /**
     * The associated message
     */
    message: string;
    /**
     * The associated public key
     */
    pub_key: Ed25519PublicKey;
    /**
     * The signature to store
     */
    sign: Ed25519Signature;
    [property: string]: any;
}

export interface WriteCells {
    /**
     * The first index to write to
     */
    begin: number;
    /**
     * The size of the data to write to storage. This is the maximum number of iterations done
     * in a string generation loop.
     */
    data_size: number;
    /**
     * The number of storage cells to write to
     */
    num_cells: number;
    [property: string]: any;
}

export interface WriteCustom {
    /**
     * The first index to write to
     */
    begin: number;
    /**
     * The content to write to the storage. Write a string to every cell from `begin`
     */
    content: string[];
    [property: string]: any;
}

/**
 * Verifies the signature stored.
 *
 * Deserializes the stored bytes into a string
 */
export enum AccessPatternMessagesEnum {
    DeserializeBytesAsString = "deserialize_bytes_as_string",
    VerifySignature = "verify_signature",
}

/**
 * Represents the available call messages for interacting with the sov-accounts module.
 *
 * Inserts a new credential id for the corresponding Account.
 */
export interface CallMessage6 {
    insert_credential_id: string;
}

/**
 * Register an attester, the parameter is the bond amount
 *
 * Register a challenger, the parameter is the bond amount
 *
 * Increases the balance of the attester.
 */
export interface CallMessage4Class {
    register_attester?:   number;
    register_challenger?: number;
    deposit_attester?:    number;
}

/**
 * Start the first phase of the two-phase exit process
 *
 * Finish the two phase exit
 *
 * Exit a challenger
 */
export enum CallMessage4Enum {
    BeginExitAttester = "begin_exit_attester",
    ExitAttester = "exit_attester",
    ExitChallenger = "exit_challenger",
}

/**
 * This enumeration represents the available call messages for interacting with the sov-bank
 * module.
 *
 * Creates a new token with the specified name and initial balance.
 *
 * Transfers a specified amount of tokens to the specified address.
 *
 * Burns a specified amount of tokens.
 *
 * Mints a specified amount of tokens.
 *
 * Freezes a token so that the supply is frozen
 *
 * Updates the list of admins for a specified token.
 */
export interface CallMessage {
    create_token?:       CreateToken;
    transfer?:           Transfer;
    burn?:               Burn;
    mint?:               Mint;
    freeze?:             Freeze;
    update_admin?:       CallMessageUpdateAdmin;
    transfer_with_memo?: TransferWithMemo;
}

export interface Burn {
    /**
     * The amount of tokens to burn.
     */
    coins: Coins;
    [property: string]: any;
}

/**
 * The amount of tokens to transfer.
 *
 * Structure that stores information specifying a given `amount` (type [`Amount`]) of coins
 * stored at a `token_id` (type [`crate::TokenId`]).
 *
 * The amount of tokens to burn.
 *
 * The amount of tokens to mint.
 */
export interface Coins {
    /**
     * The number of tokens
     */
    amount: number;
    /**
     * The ID of the token
     */
    token_id: string;
    [property: string]: any;
}

export interface CreateToken {
    /**
     * Admins list.
     */
    admins: MultiAddressEvmSolana[];
    /**
     * The initial balance of the new token.
     */
    initial_balance: number;
    /**
     * The address of the account that the new tokens are minted to.
     */
    mint_to_address: MultiAddressEvmSolana;
    /**
     * The supply cap of the new token, if any.
     */
    supply_cap?: number | null;
    /**
     * The number of decimal places this token's amounts will have.
     */
    token_decimals?: number | null;
    /**
     * The name of the new token.
     */
    token_name: string;
    [property: string]: any;
}

export interface Freeze {
    /**
     * Address of the token to be frozen
     */
    token_id: string;
    [property: string]: any;
}

export interface Mint {
    /**
     * The amount of tokens to mint.
     */
    coins: Coins;
    /**
     * Address to mint tokens to
     */
    mint_to_address: MultiAddressEvmSolana;
    [property: string]: any;
}

export interface Transfer {
    /**
     * The amount of tokens to transfer.
     */
    coins: Coins;
    /**
     * The address to which the tokens will be transferred.
     */
    to: MultiAddressEvmSolana;
    [property: string]: any;
}

export interface TransferWithMemo {
    /**
     * The amount of tokens to transfer.
     */
    coins: Coins;
    /**
     * The message included with the transfer
     */
    memo: string;
    /**
     * The address to which the tokens will be transferred.
     */
    to: MultiAddressEvmSolana;
    [property: string]: any;
}

export interface CallMessageUpdateAdmin {
    /**
     * The new admin address. If `None`, the current admin entry for the transaction sender will
     * be removed.
     */
    new_admin?: NewAdminClass | null;
    /**
     * The ID of the token whose admin list is being updated.
     */
    token_id: string;
    [property: string]: any;
}

/**
 * A standard address derived from a SHA-256 hash of a public key.
 *
 * A 20-byte Ethereum address.
 *
 * A 32-byte Solana-style base58 address.
 */
export interface NewAdminClass {
    Standard?: string;
    Evm?:      string;
    Solana?:   string;
}

/**
 * Sets the current time.
 */
export interface CallMessage7Class {
    SetOracleTime: SetOracleTime;
}

export interface SetOracleTime {
    /**
     * The new time in milliseconds since the epoch
     */
    milliseconds_since_epoch: number;
    [property: string]: any;
}

/**
 * Terminates setup mode as of the next rollup block.
 */
export enum CallMessage7Enum {
    TerminateSetupMode = "TerminateSetupMode",
}

/**
 * EVM call message.
 *
 * RLP encoded transaction.
 *
 * Update the runtime configuration
 */
export interface CallMessageClass {
    call?:                  RlpEvmTransaction;
    update_runtime_config?: EvmRuntimeConfigUpdate;
}

/**
 * RLP encoded evm transaction.
 */
export interface RlpEvmTransaction {
    /**
     * Rlp data.
     */
    rlp: number[];
    [property: string]: any;
}

/**
 * An update to the runtime configuration.
 */
export interface EvmRuntimeConfigUpdate {
    /**
     * A new chain spec to apply. None means "no change"
     */
    chain_spec_update?: null | ChainSpecUpdate;
    /**
     * A new admin address to set. None means "no change"
     */
    new_admin?: NewAdminClass | null;
    /**
     * A new contract creation policy to apply. None means "no change"
     */
    new_contract_creation_policy?: NewContractCreationPolicyClass | NewContractCreationPolicyEnum | null;
    /**
     * A new hardfork to activate and the block number at which it activates
     */
    new_hardfork?: Array<number | string> | null;
    [property: string]: any;
}

/**
 * An update to the chain spec.
 */
export interface ChainSpecUpdate {
    /**
     * The new block gas limit. Must be greater than 5M to avoid censorship. None means "no
     * change" Check that the limit is greater than 5M to avoid accidental complete shutdown.
     */
    new_block_gas_limit?: number | null;
    /**
     * The new limit for contract code size. None means "no change"
     */
    new_limit_contract_code_size?: number | null;
    /**
     * The new tx gas limit. Must be less than or equal to the effective block gas limit after
     * applying the update. None means "no change"
     */
    new_tx_gas_limit?: number | null;
    [property: string]: any;
}

/**
 * Only allowed addresses can create contracts
 */
export interface NewContractCreationPolicyClass {
    allowlist: Allowlist;
}

export interface Allowlist {
    /**
     * Addresses to add to the allowlist
     */
    add: string[];
    /**
     * Addresses to remove from the allowlist
     */
    remove: string[];
    [property: string]: any;
}

/**
 * No restrictions on contract creation
 */
export enum NewContractCreationPolicyEnum {
    Everyone = "everyone",
}

/**
 * This enumeration represents the available call messages for interacting with the
 * sov-operator-incentives module.
 */
export interface CallMessage3 {
    update_reward_address: UpdateRewardAddress;
}

export interface UpdateRewardAddress {
    /**
     * The new address that will receive rewards for operating the rollup. Note: We do not
     * verify possession of the corresponding private key, so it's possible to set an address
     * for which the `sender` does not control the private key.
     */
    new_reward_address: MultiAddressEvmSolana;
    [property: string]: any;
}

/**
 * Call messages for interacting with the `Paymaster` module.
 *
 * ## Note: These call messages are highly unusual in that they have different effects based
 * on the address of the sequencer who places them on chain. See the docs on individual
 * variants for more information.
 *
 * Register a new payer with the given policy. If the sequencer who places this message on
 * chain is present in the list of `authorized_sequencers` to use the payer, the payer
 * address for that sequencer is set to the address of the newly registered payer.
 *
 * Set the payer address for the sequencer to the given address. This call message is highly
 * unusual in that it executes regardless of the sender address on the rollup. Sequencers
 * who do not wish to update their payer address should not sequence transactions containing
 * this callmessage.
 *
 * Update the policy for a given payer. If the sequencer who places this message on chain is
 * present in the list of `authorized_sequencers` to use the payer after the update, the
 * payer address for that sequencer is set to the address of the newly registered paymaster.
 */
export interface CallMessage8 {
    register_paymaster?:      RegisterPaymaster;
    set_payer_for_sequencer?: SetPayerForSequencer;
    update_policy?:           UpdatePolicy;
}

export interface RegisterPaymaster {
    policy: PaymasterPolicyInitializer;
    [property: string]: any;
}

/**
 * An initial policy for a paymaster. This includes... - A set of sequencers that can use
 * the paymaster - A set of users authorized to update this policy - A default policy for
 * accepting/rejecting gas requests - Specific policies for accepting/rejecting gas requests
 * from particular users
 */
export interface PaymasterPolicyInitializer {
    /**
     * Sequencers who are authorized to use this payer.
     */
    authorized_sequencers: AuthorizedSequencersClass | AuthorizedSequencersEnum;
    /**
     * Users who are authorized to update this policy.
     */
    authorized_updaters: MultiAddressEvmSolana[];
    /**
     * Default payee policy for users that are not in the balances map.
     */
    default_payee_policy: PayeePolicyClass | PayeePolicyEnum;
    /**
     * A mapping from user address to the policy for that user.
     */
    payees: Array<Array<SafeVec20_OfTupleOfMultiAddressEvmSolanaAndPayeePolicyClass | PayeePolicyEnum>>;
    [property: string]: any;
}

/**
 * Only the specified sequencers may use this payer.
 */
export interface AuthorizedSequencersClass {
    some: string[];
}

/**
 * All sequencers are authorized to use this payer (according to its policy).
 */
export enum AuthorizedSequencersEnum {
    All = "all",
}

/**
 * The paymaster pays the fees for a particular sender when the policy allows it... - If the
 * policy specifies a `max_fee`, the transaction's max fee must be less than or equal to
 * that value - if the policy specifies a `max_gas_price`, the current gas price must be
 * less than or equal to that value - If the policy specifies a gas limit, the transaction
 * must also specify a limit *and* that limit must be less than or equal to `gas_limit`.
 *
 * - If the policy specifies a transaction_limit, the policy can only cover that many
 * transactions, after which it will expire and be replaced with a Deny policy
 *
 * In all other cases, the sender pays their own fees.
 */
export interface PayeePolicyClass {
    allow: Allow;
}

export interface Allow {
    gas_limit?:         number[] | null;
    max_fee?:           number | null;
    max_gas_price?:     number[] | null;
    transaction_limit?: number | null;
    [property: string]: any;
}

/**
 * The payer does not pay fees for any transaction using this policy.
 */
export enum PayeePolicyEnum {
    Deny = "deny",
}

/**
 * A standard address derived from a SHA-256 hash of a public key.
 *
 * A 20-byte Ethereum address.
 *
 * A 32-byte Solana-style base58 address.
 *
 * The paymaster pays the fees for a particular sender when the policy allows it... - If the
 * policy specifies a `max_fee`, the transaction's max fee must be less than or equal to
 * that value - if the policy specifies a `max_gas_price`, the current gas price must be
 * less than or equal to that value - If the policy specifies a gas limit, the transaction
 * must also specify a limit *and* that limit must be less than or equal to `gas_limit`.
 *
 * - If the policy specifies a transaction_limit, the policy can only cover that many
 * transactions, after which it will expire and be replaced with a Deny policy
 *
 * In all other cases, the sender pays their own fees.
 */
export interface SafeVec20_OfTupleOfMultiAddressEvmSolanaAndPayeePolicyClass {
    Standard?: string;
    Evm?:      string;
    Solana?:   string;
    allow?:    Allow;
}

export interface SetPayerForSequencer {
    payer: MultiAddressEvmSolana;
    [property: string]: any;
}

export interface UpdatePolicy {
    payer:  MultiAddressEvmSolana;
    update: PolicyUpdate;
    [property: string]: any;
}

/**
 * An update to the policy of a single gas payer
 */
export interface PolicyUpdate {
    default_policy?:           PayeePolicyClass | PayeePolicyEnum | null;
    payee_policies_to_delete?: MultiAddressEvmSolana[] | null;
    payee_policies_to_set?:    Array<Array<SafeVec20_OfTupleOfMultiAddressEvmSolanaAndPayeePolicyClass | PayeePolicyEnum>> | null;
    sequencer_update?:         SequencerUpdateClass | SequencerUpdateEnum | null;
    updaters_to_add?:          MultiAddressEvmSolana[] | null;
    updaters_to_remove?:       MultiAddressEvmSolana[] | null;
    [property: string]: any;
}

/**
 * Sets the list of authorized sequencers to an explicit whitelist if it was previously
 * `AllowAll`. Adds and removes the requested addresses from the sequencer whitelist.
 */
export interface SequencerUpdateClass {
    update: SequencerUpdateList;
}

/**
 * A list of updates to the `allowed_sequencers` list for a particular payer.
 */
export interface SequencerUpdateList {
    to_add?:    string[] | null;
    to_remove?: string[] | null;
    [property: string]: any;
}

/**
 * Authorizes any sequencer to use this payer.
 */
export enum SequencerUpdateEnum {
    AllowAll = "allow_all",
}

/**
 * Add a new prover as a bonded prover.
 *
 * Increases the balance of the prover, transferring the funds from the prover account to
 * the rollup.
 */
export interface CallMessage5Class {
    register?: number;
    deposit?:  number;
}

/**
 * Unbonds the prover.
 */
export enum CallMessage5Enum {
    Exit = "exit",
}

/**
 * This enumeration represents the available call messages for interacting with the
 * `sov-sequencer-registry` module.
 *
 * Add a new sequencer to the sequencer registry.
 *
 * Increases the balance of the sequencer, transferring the funds from the sequencer account
 * to the rollup.
 *
 * Initiate a withdrawal of a sequencer's balance.
 *
 * Withdraw a sequencer's balance after waiting for the withdrawal period.
 */
export interface CallMessage2 {
    register?:            Register;
    deposit?:             Deposit;
    initiate_withdrawal?: InitiateWithdrawal;
    withdraw?:            Withdraw;
}

export interface Deposit {
    /**
     * The amount to increase.
     */
    amount: number;
    /**
     * The DA address of the sequencer.
     */
    da_address: string;
    [property: string]: any;
}

export interface InitiateWithdrawal {
    /**
     * The DA address of the sequencer you're removing.
     */
    da_address: string;
    [property: string]: any;
}

export interface Register {
    /**
     * The initial balance of the sequencer.
     */
    amount: number;
    /**
     * The Da address of the sequencer you're registering.
     */
    da_address: string;
    [property: string]: any;
}

export interface Withdraw {
    /**
     * The DA address of the sequencer you're removing.
     */
    da_address: string;
    [property: string]: any;
}

/**
 * This enumeration represents the available call messages for interacting with the module.
 *
 * Read and set many individual values.
 *
 * Read and set entries in a large vector stored as a `StateValue`
 *
 * Run CPU heavy operation. Each iteration computes a hash with the Spec::Hasher.
 */
export interface CallMessage9 {
    read_and_set_many_individual_values?: ReadAndSetManyIndividualValues;
    read_and_set_heavy_state?:            ReadAndSetHeavyState;
    run_c_p_u_heavy_operation?:           RunCPUHeavyOperation;
}

export interface ReadAndSetHeavyState {
    /**
     * The max size of the heavy state.
     */
    max_heavy_state_size: number;
    /**
     * The number of new values to read and set.
     */
    number_of_new_values: number;
    /**
     * The salt.
     */
    salt: number;
    [property: string]: any;
}

export interface ReadAndSetManyIndividualValues {
    /**
     * The number of values to read and set.
     */
    number_of_operations: number;
    /**
     * The salt.
     */
    salt: number;
    [property: string]: any;
}

export interface RunCPUHeavyOperation {
    /**
     * The number of iterations.
     */
    iterations: number;
    [property: string]: any;
}

// Converts JSON strings to/from your types
// and asserts the results of JSON.parse at runtime
export class Convert {
    public static toRuntimeCall(json: string): RuntimeCall {
        return cast(JSON.parse(json), r("RuntimeCall"));
    }

    public static runtimeCallToJson(value: RuntimeCall): string {
        return JSON.stringify(uncast(value, r("RuntimeCall")), null, 2);
    }
}

function invalidValue(typ: any, val: any, key: any, parent: any = ''): never {
    const prettyTyp = prettyTypeName(typ);
    const parentText = parent ? ` on ${parent}` : '';
    const keyText = key ? ` for key "${key}"` : '';
    throw Error(`Invalid value${keyText}${parentText}. Expected ${prettyTyp} but got ${JSON.stringify(val)}`);
}

function prettyTypeName(typ: any): string {
    if (Array.isArray(typ)) {
        if (typ.length === 2 && typ[0] === undefined) {
            return `an optional ${prettyTypeName(typ[1])}`;
        } else {
            return `one of [${typ.map(a => { return prettyTypeName(a); }).join(", ")}]`;
        }
    } else if (typeof typ === "object" && typ.literal !== undefined) {
        return typ.literal;
    } else {
        return typeof typ;
    }
}

function jsonToJSProps(typ: any): any {
    if (typ.jsonToJS === undefined) {
        const map: any = {};
        typ.props.forEach((p: any) => map[p.json] = { key: p.js, typ: p.typ });
        typ.jsonToJS = map;
    }
    return typ.jsonToJS;
}

function jsToJSONProps(typ: any): any {
    if (typ.jsToJSON === undefined) {
        const map: any = {};
        typ.props.forEach((p: any) => map[p.js] = { key: p.json, typ: p.typ });
        typ.jsToJSON = map;
    }
    return typ.jsToJSON;
}

function transform(val: any, typ: any, getProps: any, key: any = '', parent: any = ''): any {
    function transformPrimitive(typ: string, val: any): any {
        if (typeof typ === typeof val) return val;
        return invalidValue(typ, val, key, parent);
    }

    function transformUnion(typs: any[], val: any): any {
        // val must validate against one typ in typs
        const l = typs.length;
        for (let i = 0; i < l; i++) {
            const typ = typs[i];
            try {
                return transform(val, typ, getProps);
            } catch (_) {}
        }
        return invalidValue(typs, val, key, parent);
    }

    function transformEnum(cases: string[], val: any): any {
        if (cases.indexOf(val) !== -1) return val;
        return invalidValue(cases.map(a => { return l(a); }), val, key, parent);
    }

    function transformArray(typ: any, val: any): any {
        // val must be an array with no invalid elements
        if (!Array.isArray(val)) return invalidValue(l("array"), val, key, parent);
        return val.map(el => transform(el, typ, getProps));
    }

    function transformDate(val: any): any {
        if (val === null) {
            return null;
        }
        const d = new Date(val);
        if (isNaN(d.valueOf())) {
            return invalidValue(l("Date"), val, key, parent);
        }
        return d;
    }

    function transformObject(props: { [k: string]: any }, additional: any, val: any): any {
        if (val === null || typeof val !== "object" || Array.isArray(val)) {
            return invalidValue(l(ref || "object"), val, key, parent);
        }
        const result: any = {};
        Object.getOwnPropertyNames(props).forEach(key => {
            const prop = props[key];
            const v = Object.prototype.hasOwnProperty.call(val, key) ? val[key] : undefined;
            result[prop.key] = transform(v, prop.typ, getProps, key, ref);
        });
        Object.getOwnPropertyNames(val).forEach(key => {
            if (!Object.prototype.hasOwnProperty.call(props, key)) {
                result[key] = transform(val[key], additional, getProps, key, ref);
            }
        });
        return result;
    }

    if (typ === "any") return val;
    if (typ === null) {
        if (val === null) return val;
        return invalidValue(typ, val, key, parent);
    }
    if (typ === false) return invalidValue(typ, val, key, parent);
    let ref: any = undefined;
    while (typeof typ === "object" && typ.ref !== undefined) {
        ref = typ.ref;
        typ = typeMap[typ.ref];
    }
    if (Array.isArray(typ)) return transformEnum(typ, val);
    if (typeof typ === "object") {
        return typ.hasOwnProperty("unionMembers") ? transformUnion(typ.unionMembers, val)
            : typ.hasOwnProperty("arrayItems")    ? transformArray(typ.arrayItems, val)
            : typ.hasOwnProperty("props")         ? transformObject(getProps(typ), typ.additional, val)
            : invalidValue(typ, val, key, parent);
    }
    // Numbers can be parsed by Date but shouldn't be.
    if (typ === Date && typeof val !== "number") return transformDate(val);
    return transformPrimitive(typ, val);
}

function cast<T>(val: any, typ: any): T {
    return transform(val, typ, jsonToJSProps);
}

function uncast<T>(val: T, typ: any): any {
    return transform(val, typ, jsToJSONProps);
}

function l(typ: any) {
    return { literal: typ };
}

function a(typ: any) {
    return { arrayItems: typ };
}

function u(...typs: any[]) {
    return { unionMembers: typs };
}

function o(props: any[], additional: any) {
    return { props, additional };
}

function m(additional: any) {
    return { props: [], additional };
}

function r(name: string) {
    return { ref: name };
}

const typeMap: any = {
    "RuntimeCall": o([
        { json: "bank", js: "bank", typ: u(undefined, r("CallMessage")) },
        { json: "sequencer_registry", js: "sequencer_registry", typ: u(undefined, r("CallMessage2")) },
        { json: "operator_incentives", js: "operator_incentives", typ: u(undefined, r("CallMessage3")) },
        { json: "attester_incentives", js: "attester_incentives", typ: u(undefined, u(r("CallMessage4Class"), r("CallMessage4Enum"))) },
        { json: "prover_incentives", js: "prover_incentives", typ: u(undefined, u(r("CallMessage5Class"), r("CallMessage5Enum"))) },
        { json: "accounts", js: "accounts", typ: u(undefined, r("CallMessage6")) },
        { json: "uniqueness", js: "uniqueness", typ: u(undefined, null) },
        { json: "chain_state", js: "chain_state", typ: u(undefined, u(r("CallMessage7Class"), r("CallMessage7Enum"))) },
        { json: "blob_storage", js: "blob_storage", typ: u(undefined, null) },
        { json: "paymaster", js: "paymaster", typ: u(undefined, r("CallMessage8")) },
        { json: "evm", js: "evm", typ: u(undefined, r("CallMessageClass")) },
        { json: "access_pattern", js: "access_pattern", typ: u(undefined, u(r("AccessPatternMessagesClass"), r("AccessPatternMessagesEnum"))) },
        { json: "synthetic_load", js: "synthetic_load", typ: u(undefined, r("CallMessage9")) },
    ], false),
    "AccessPatternMessagesClass": o([
        { json: "write_cells", js: "write_cells", typ: u(undefined, r("WriteCells")) },
        { json: "write_custom", js: "write_custom", typ: u(undefined, r("WriteCustom")) },
        { json: "read_cells", js: "read_cells", typ: u(undefined, r("ReadCells")) },
        { json: "hash_bytes", js: "hash_bytes", typ: u(undefined, r("HashBytes")) },
        { json: "hash_custom", js: "hash_custom", typ: u(undefined, r("HashCustom")) },
        { json: "store_signature", js: "store_signature", typ: u(undefined, r("StoreSignature")) },
        { json: "verify_custom_signature", js: "verify_custom_signature", typ: u(undefined, r("VerifyCustomSignature")) },
        { json: "store_serialized_string", js: "store_serialized_string", typ: u(undefined, r("StoreSerializedString")) },
        { json: "deserialize_custom_string", js: "deserialize_custom_string", typ: u(undefined, r("DeserializeCustomString")) },
        { json: "delete_cells", js: "delete_cells", typ: u(undefined, r("DeleteCells")) },
        { json: "set_hook", js: "set_hook", typ: u(undefined, r("SetHook")) },
        { json: "update_admin", js: "update_admin", typ: u(undefined, r("AccessPatternMessagesUpdateAdmin")) },
    ], false),
    "DeleteCells": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "num_cells", js: "num_cells", typ: 0 },
    ], "any"),
    "DeserializeCustomString": o([
        { json: "input", js: "input", typ: a(0) },
    ], "any"),
    "HashBytes": o([
        { json: "filler", js: "filler", typ: 0 },
        { json: "size", js: "size", typ: 0 },
    ], "any"),
    "HashCustom": o([
        { json: "input", js: "input", typ: a(0) },
    ], "any"),
    "ReadCells": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "num_cells", js: "num_cells", typ: 0 },
    ], "any"),
    "SetHook": o([
        { json: "post", js: "post", typ: u(undefined, u(a(r("HooksConfig")), null)) },
        { json: "pre", js: "pre", typ: u(undefined, u(a(r("HooksConfig")), null)) },
    ], "any"),
    "HooksConfig": o([
        { json: "Read", js: "Read", typ: u(undefined, r("Read")) },
        { json: "Write", js: "Write", typ: u(undefined, r("Write")) },
        { json: "Delete", js: "Delete", typ: u(undefined, r("Delete")) },
    ], false),
    "Delete": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "size", js: "size", typ: 0 },
    ], "any"),
    "Read": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "size", js: "size", typ: 0 },
    ], "any"),
    "Write": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "data_size", js: "data_size", typ: 0 },
        { json: "size", js: "size", typ: 0 },
    ], "any"),
    "StoreSerializedString": o([
        { json: "input", js: "input", typ: a(0) },
    ], "any"),
    "StoreSignature": o([
        { json: "message", js: "message", typ: "" },
        { json: "pub_key", js: "pub_key", typ: r("Ed25519PublicKey") },
        { json: "sign", js: "sign", typ: r("Ed25519Signature") },
    ], "any"),
    "Ed25519PublicKey": o([
        { json: "pub_key", js: "pub_key", typ: a(0) },
    ], "any"),
    "Ed25519Signature": o([
        { json: "bytes", js: "bytes", typ: a(0) },
        { json: "msg_sig", js: "msg_sig", typ: a(0) },
    ], "any"),
    "AccessPatternMessagesUpdateAdmin": o([
        { json: "new_admin", js: "new_admin", typ: r("MultiAddressEvmSolana") },
    ], "any"),
    "MultiAddressEvmSolana": o([
        { json: "Standard", js: "Standard", typ: u(undefined, "") },
        { json: "Evm", js: "Evm", typ: u(undefined, "") },
        { json: "Solana", js: "Solana", typ: u(undefined, "") },
    ], false),
    "VerifyCustomSignature": o([
        { json: "message", js: "message", typ: "" },
        { json: "pub_key", js: "pub_key", typ: r("Ed25519PublicKey") },
        { json: "sign", js: "sign", typ: r("Ed25519Signature") },
    ], "any"),
    "WriteCells": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "data_size", js: "data_size", typ: 0 },
        { json: "num_cells", js: "num_cells", typ: 0 },
    ], "any"),
    "WriteCustom": o([
        { json: "begin", js: "begin", typ: 0 },
        { json: "content", js: "content", typ: a("") },
    ], "any"),
    "CallMessage6": o([
        { json: "insert_credential_id", js: "insert_credential_id", typ: "" },
    ], false),
    "CallMessage4Class": o([
        { json: "register_attester", js: "register_attester", typ: u(undefined, 0) },
        { json: "register_challenger", js: "register_challenger", typ: u(undefined, 0) },
        { json: "deposit_attester", js: "deposit_attester", typ: u(undefined, 0) },
    ], false),
    "CallMessage": o([
        { json: "create_token", js: "create_token", typ: u(undefined, r("CreateToken")) },
        { json: "transfer", js: "transfer", typ: u(undefined, r("Transfer")) },
        { json: "burn", js: "burn", typ: u(undefined, r("Burn")) },
        { json: "mint", js: "mint", typ: u(undefined, r("Mint")) },
        { json: "freeze", js: "freeze", typ: u(undefined, r("Freeze")) },
        { json: "update_admin", js: "update_admin", typ: u(undefined, r("CallMessageUpdateAdmin")) },
        { json: "transfer_with_memo", js: "transfer_with_memo", typ: u(undefined, r("TransferWithMemo")) },
    ], false),
    "Burn": o([
        { json: "coins", js: "coins", typ: r("Coins") },
    ], "any"),
    "Coins": o([
        { json: "amount", js: "amount", typ: 0 },
        { json: "token_id", js: "token_id", typ: "" },
    ], "any"),
    "CreateToken": o([
        { json: "admins", js: "admins", typ: a(r("MultiAddressEvmSolana")) },
        { json: "initial_balance", js: "initial_balance", typ: 0 },
        { json: "mint_to_address", js: "mint_to_address", typ: r("MultiAddressEvmSolana") },
        { json: "supply_cap", js: "supply_cap", typ: u(undefined, u(0, null)) },
        { json: "token_decimals", js: "token_decimals", typ: u(undefined, u(0, null)) },
        { json: "token_name", js: "token_name", typ: "" },
    ], "any"),
    "Freeze": o([
        { json: "token_id", js: "token_id", typ: "" },
    ], "any"),
    "Mint": o([
        { json: "coins", js: "coins", typ: r("Coins") },
        { json: "mint_to_address", js: "mint_to_address", typ: r("MultiAddressEvmSolana") },
    ], "any"),
    "Transfer": o([
        { json: "coins", js: "coins", typ: r("Coins") },
        { json: "to", js: "to", typ: r("MultiAddressEvmSolana") },
    ], "any"),
    "TransferWithMemo": o([
        { json: "coins", js: "coins", typ: r("Coins") },
        { json: "memo", js: "memo", typ: "" },
        { json: "to", js: "to", typ: r("MultiAddressEvmSolana") },
    ], "any"),
    "CallMessageUpdateAdmin": o([
        { json: "new_admin", js: "new_admin", typ: u(undefined, u(r("NewAdminClass"), null)) },
        { json: "token_id", js: "token_id", typ: "" },
    ], "any"),
    "NewAdminClass": o([
        { json: "Standard", js: "Standard", typ: u(undefined, "") },
        { json: "Evm", js: "Evm", typ: u(undefined, "") },
        { json: "Solana", js: "Solana", typ: u(undefined, "") },
    ], false),
    "CallMessage7Class": o([
        { json: "SetOracleTime", js: "SetOracleTime", typ: r("SetOracleTime") },
    ], false),
    "SetOracleTime": o([
        { json: "milliseconds_since_epoch", js: "milliseconds_since_epoch", typ: 0 },
    ], "any"),
    "CallMessageClass": o([
        { json: "call", js: "call", typ: u(undefined, r("RlpEvmTransaction")) },
        { json: "update_runtime_config", js: "update_runtime_config", typ: u(undefined, r("EvmRuntimeConfigUpdate")) },
    ], false),
    "RlpEvmTransaction": o([
        { json: "rlp", js: "rlp", typ: a(0) },
    ], "any"),
    "EvmRuntimeConfigUpdate": o([
        { json: "chain_spec_update", js: "chain_spec_update", typ: u(undefined, u(null, r("ChainSpecUpdate"))) },
        { json: "new_admin", js: "new_admin", typ: u(undefined, u(r("NewAdminClass"), null)) },
        { json: "new_contract_creation_policy", js: "new_contract_creation_policy", typ: u(undefined, u(r("NewContractCreationPolicyClass"), r("NewContractCreationPolicyEnum"), null)) },
        { json: "new_hardfork", js: "new_hardfork", typ: u(undefined, u(a(u(0, "")), null)) },
    ], "any"),
    "ChainSpecUpdate": o([
        { json: "new_block_gas_limit", js: "new_block_gas_limit", typ: u(undefined, u(0, null)) },
        { json: "new_limit_contract_code_size", js: "new_limit_contract_code_size", typ: u(undefined, u(0, null)) },
        { json: "new_tx_gas_limit", js: "new_tx_gas_limit", typ: u(undefined, u(0, null)) },
    ], "any"),
    "NewContractCreationPolicyClass": o([
        { json: "allowlist", js: "allowlist", typ: r("Allowlist") },
    ], false),
    "Allowlist": o([
        { json: "add", js: "add", typ: a("") },
        { json: "remove", js: "remove", typ: a("") },
    ], "any"),
    "CallMessage3": o([
        { json: "update_reward_address", js: "update_reward_address", typ: r("UpdateRewardAddress") },
    ], false),
    "UpdateRewardAddress": o([
        { json: "new_reward_address", js: "new_reward_address", typ: r("MultiAddressEvmSolana") },
    ], "any"),
    "CallMessage8": o([
        { json: "register_paymaster", js: "register_paymaster", typ: u(undefined, r("RegisterPaymaster")) },
        { json: "set_payer_for_sequencer", js: "set_payer_for_sequencer", typ: u(undefined, r("SetPayerForSequencer")) },
        { json: "update_policy", js: "update_policy", typ: u(undefined, r("UpdatePolicy")) },
    ], false),
    "RegisterPaymaster": o([
        { json: "policy", js: "policy", typ: r("PaymasterPolicyInitializer") },
    ], "any"),
    "PaymasterPolicyInitializer": o([
        { json: "authorized_sequencers", js: "authorized_sequencers", typ: u(r("AuthorizedSequencersClass"), r("AuthorizedSequencersEnum")) },
        { json: "authorized_updaters", js: "authorized_updaters", typ: a(r("MultiAddressEvmSolana")) },
        { json: "default_payee_policy", js: "default_payee_policy", typ: u(r("PayeePolicyClass"), r("PayeePolicyEnum")) },
        { json: "payees", js: "payees", typ: a(a(u(r("SafeVec20_OfTupleOfMultiAddressEvmSolanaAndPayeePolicyClass"), r("PayeePolicyEnum")))) },
    ], "any"),
    "AuthorizedSequencersClass": o([
        { json: "some", js: "some", typ: a("") },
    ], false),
    "PayeePolicyClass": o([
        { json: "allow", js: "allow", typ: r("Allow") },
    ], false),
    "Allow": o([
        { json: "gas_limit", js: "gas_limit", typ: u(undefined, u(a(3.14), null)) },
        { json: "max_fee", js: "max_fee", typ: u(undefined, u(0, null)) },
        { json: "max_gas_price", js: "max_gas_price", typ: u(undefined, u(a(3.14), null)) },
        { json: "transaction_limit", js: "transaction_limit", typ: u(undefined, u(0, null)) },
    ], "any"),
    "SafeVec20_OfTupleOfMultiAddressEvmSolanaAndPayeePolicyClass": o([
        { json: "Standard", js: "Standard", typ: u(undefined, "") },
        { json: "Evm", js: "Evm", typ: u(undefined, "") },
        { json: "Solana", js: "Solana", typ: u(undefined, "") },
        { json: "allow", js: "allow", typ: u(undefined, r("Allow")) },
    ], false),
    "SetPayerForSequencer": o([
        { json: "payer", js: "payer", typ: r("MultiAddressEvmSolana") },
    ], "any"),
    "UpdatePolicy": o([
        { json: "payer", js: "payer", typ: r("MultiAddressEvmSolana") },
        { json: "update", js: "update", typ: r("PolicyUpdate") },
    ], "any"),
    "PolicyUpdate": o([
        { json: "default_policy", js: "default_policy", typ: u(undefined, u(r("PayeePolicyClass"), r("PayeePolicyEnum"), null)) },
        { json: "payee_policies_to_delete", js: "payee_policies_to_delete", typ: u(undefined, u(a(r("MultiAddressEvmSolana")), null)) },
        { json: "payee_policies_to_set", js: "payee_policies_to_set", typ: u(undefined, u(a(a(u(r("SafeVec20_OfTupleOfMultiAddressEvmSolanaAndPayeePolicyClass"), r("PayeePolicyEnum")))), null)) },
        { json: "sequencer_update", js: "sequencer_update", typ: u(undefined, u(r("SequencerUpdateClass"), r("SequencerUpdateEnum"), null)) },
        { json: "updaters_to_add", js: "updaters_to_add", typ: u(undefined, u(a(r("MultiAddressEvmSolana")), null)) },
        { json: "updaters_to_remove", js: "updaters_to_remove", typ: u(undefined, u(a(r("MultiAddressEvmSolana")), null)) },
    ], "any"),
    "SequencerUpdateClass": o([
        { json: "update", js: "update", typ: r("SequencerUpdateList") },
    ], false),
    "SequencerUpdateList": o([
        { json: "to_add", js: "to_add", typ: u(undefined, u(a(""), null)) },
        { json: "to_remove", js: "to_remove", typ: u(undefined, u(a(""), null)) },
    ], "any"),
    "CallMessage5Class": o([
        { json: "register", js: "register", typ: u(undefined, 0) },
        { json: "deposit", js: "deposit", typ: u(undefined, 0) },
    ], false),
    "CallMessage2": o([
        { json: "register", js: "register", typ: u(undefined, r("Register")) },
        { json: "deposit", js: "deposit", typ: u(undefined, r("Deposit")) },
        { json: "initiate_withdrawal", js: "initiate_withdrawal", typ: u(undefined, r("InitiateWithdrawal")) },
        { json: "withdraw", js: "withdraw", typ: u(undefined, r("Withdraw")) },
    ], false),
    "Deposit": o([
        { json: "amount", js: "amount", typ: 0 },
        { json: "da_address", js: "da_address", typ: "" },
    ], "any"),
    "InitiateWithdrawal": o([
        { json: "da_address", js: "da_address", typ: "" },
    ], "any"),
    "Register": o([
        { json: "amount", js: "amount", typ: 0 },
        { json: "da_address", js: "da_address", typ: "" },
    ], "any"),
    "Withdraw": o([
        { json: "da_address", js: "da_address", typ: "" },
    ], "any"),
    "CallMessage9": o([
        { json: "read_and_set_many_individual_values", js: "read_and_set_many_individual_values", typ: u(undefined, r("ReadAndSetManyIndividualValues")) },
        { json: "read_and_set_heavy_state", js: "read_and_set_heavy_state", typ: u(undefined, r("ReadAndSetHeavyState")) },
        { json: "run_c_p_u_heavy_operation", js: "run_c_p_u_heavy_operation", typ: u(undefined, r("RunCPUHeavyOperation")) },
    ], false),
    "ReadAndSetHeavyState": o([
        { json: "max_heavy_state_size", js: "max_heavy_state_size", typ: 0 },
        { json: "number_of_new_values", js: "number_of_new_values", typ: 0 },
        { json: "salt", js: "salt", typ: 0 },
    ], "any"),
    "ReadAndSetManyIndividualValues": o([
        { json: "number_of_operations", js: "number_of_operations", typ: 0 },
        { json: "salt", js: "salt", typ: 0 },
    ], "any"),
    "RunCPUHeavyOperation": o([
        { json: "iterations", js: "iterations", typ: 0 },
    ], "any"),
    "AccessPatternMessagesEnum": [
        "deserialize_bytes_as_string",
        "verify_signature",
    ],
    "CallMessage4Enum": [
        "begin_exit_attester",
        "exit_attester",
        "exit_challenger",
    ],
    "CallMessage7Enum": [
        "TerminateSetupMode",
    ],
    "NewContractCreationPolicyEnum": [
        "everyone",
    ],
    "AuthorizedSequencersEnum": [
        "all",
    ],
    "PayeePolicyEnum": [
        "deny",
    ],
    "SequencerUpdateEnum": [
        "allow_all",
    ],
    "CallMessage5Enum": [
        "exit",
    ],
};
