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
    accounts?:                 CallMessage;
    uniqueness?:               null;
    bank?:                     CallMessage2;
    sequencer_registry?:       CallMessage3;
    operator_incentives?:      CallMessage4;
    attester_incentives?:      CallMessage5Class | CallMessage5Enum;
    prover_incentives?:        CallMessage6Class | CallMessage6Enum;
    value_setter?:           CallMessage7;
    chain_state?:              null;
    blob_storage?:             null;
    paymaster?:                CallMessage8;
    mailbox?:                  CallMessage9;
    interchain_gas_paymaster?: CallMessage10;
    merkle_tree_hook?:         null;
    warp?:                     CallMessageForConfigurableSpec;
}

/**
 * Represents the available call messages for interacting with the sov-accounts module.
 *
 * Inserts a new credential id for the corresponding Account.
 */
export interface CallMessage {
    insert_credential_id: string;
}

/**
 * Register an attester, the parameter is the bond amount
 *
 * Register a challenger, the parameter is the bond amount
 *
 * Increases the balance of the attester.
 */
export interface CallMessage5Class {
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
export enum CallMessage5Enum {
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
 */
export interface CallMessage2 {
    create_token?: CreateToken;
    transfer?:     Transfer;
    burn?:         Burn;
    mint?:         Mint;
    freeze?:       Freeze;
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
    admins: string[];
    /**
     * The initial balance of the new token.
     */
    initial_balance: number;
    /**
     * The address of the account that the new tokens are minted to.
     */
    mint_to_address: string;
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
    mint_to_address: string;
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
    to: string;
    [property: string]: any;
}

/**
 * This enumeration represents the available call messages for interacting with the
 * `ValueSetter` module. The `derive` for [`schemars::JsonSchema`] is a requirement of
 * [`sov_modules_api::ModuleCallJsonSchema`].
 */
export interface CallMessage7 {
    set_value: number;
}

/**
 * InterchainGasPaymaster CallMessage
 *
 * Set or update config for relayer (sender).
 *
 * This could be used to clear values too.
 *
 * Update oracle data for relayer (sender)
 *
 * Beneficiary (sender) claim all relayer rewards.
 */
export interface CallMessage10 {
    set_relayer_config?: SetRelayerConfig;
    update_oracle_data?: UpdateOracleData;
    claim_rewards?:      ClaimRewards;
}

export interface ClaimRewards {
    /**
     * Relayer to transfer tokens from.
     */
    relayer_address: string;
    [property: string]: any;
}

export interface SetRelayerConfig {
    /**
     * Beneficiary who can claim relayer rewards.
     */
    beneficiary?: null | string;
    /**
     * Default gas used if custom one is not set.
     */
    default_gas: number;
    /**
     * Custom default gas per domain.
     */
    domain_default_gas: DomainDefaultGas[];
    /**
     * oracle data per domain.
     */
    domain_oracle_data: DomainOracleData[];
    [property: string]: any;
}

/**
 * Domain Default Gas used in `CallMessage::SetRelayerConfig`.
 */
export interface DomainDefaultGas {
    /**
     * Default gas.
     */
    default_gas: number;
    /**
     * Domain.
     */
    domain: number;
    [property: string]: any;
}

/**
 * Domain Oracle Data used in `CallMessage::SetRelayerConfig`.
 */
export interface DomainOracleData {
    /**
     * Oracle data value.
     */
    data_value: ExchangeRateAndGasPrice;
    /**
     * Domain.
     */
    domain: number;
    [property: string]: any;
}

/**
 * Oracle data value.
 *
 * Oracle data used to calculate required gas.
 *
 * Oracle data.
 *
 * Relayer is responsible to multiple token_rate_exchange by `TOKEN_EXCHANGE_RATE_SCALE`.
 */
export interface ExchangeRateAndGasPrice {
    /**
     * Gas price.
     */
    gas_price: number;
    /**
     * Token exchange rate, calculated as local gas token price / remote gas token price.
     *
     * Relayer is responsible to multiply token_rate_exchange by `TOKEN_EXCHANGE_RATE_SCALE`.
     */
    token_exchange_rate: number;
    [property: string]: any;
}

export interface UpdateOracleData {
    /**
     * Domain or destination domain (i.e. chain id in hyperlane).
     */
    domain: number;
    /**
     * Oracle data.
     *
     * Relayer is responsible to multiple token_rate_exchange by `TOKEN_EXCHANGE_RATE_SCALE`.
     */
    oracle_data: ExchangeRateAndGasPrice;
    [property: string]: any;
}

/**
 * This enumeration represents the available call messages for interacting with the
 * `sov-value-setter` module.
 *
 * Sends an outbound message to the specified recipient.
 *
 * Receive an inbound message. This is called *on the desitination chain* by the relayer
 * after a `dispatch` call has been made on the source chain.
 *
 * Passes the message metadata and body to the security module (ISM) for verification, then
 * calls the recipient's `handle` function with the message body.
 *
 * Announce a validator and its signatures' storage.
 */
export interface CallMessage9 {
    dispatch?: Dispatch;
    process?:  Process;
    announce?: Announce;
}

export interface Announce {
    /**
     * Signature of the announcement message for verification.
     */
    signature: string;
    /**
     * Location of validator's signatures.
     */
    storage_location: string;
    /**
     * Address of a validator.
     */
    validator_address: string;
    [property: string]: any;
}

export interface Dispatch {
    /**
     * The message body. For example, if the recipient is a warp route, this will encode the
     * amount/type of funds being transferred
     */
    body: string;
    /**
     * The destination domain (aka "Chain ID")
     */
    domain: number;
    /**
     * A limit for the payment to relayer to cover gas needed for message delivery. If relayer
     * demands more than this value of native gas token, dispatching message will fail. If it
     * demands less than this, only needed amount will be paid.
     */
    gas_payment_limit: number;
    /**
     * The "metadata" which is used to verify the message or control hooks. Can be used to set
     * the destination gas limit for a message using [`IGPMetadata`](crate::igp::IGPMetadata)
     */
    metadata?: null | string;
    /**
     * The recipient address. Must implement the `handle` function - i.e. be a smart contract
     */
    recipient: string;
    /**
     * Selected relayer
     */
    relayer?: null | string;
    [property: string]: any;
}

export interface Process {
    /**
     * The serialized [`Message`] struct
     */
    message: string;
    /**
     * Metadata used to verify the message.
     */
    metadata: string;
    [property: string]: any;
}

/**
 * This enumeration represents the available call messages for interacting with the
 * sov-operator-incentives module.
 */
export interface CallMessage4 {
    update_reward_address: UpdateRewardAddress;
}

export interface UpdateRewardAddress {
    /**
     * The new address that will receive rewards for operating the rollup. Note: We do not
     * verify possession of the corresponding private key, so it's possible to set an address
     * for which the `sender` does not control the private key.
     */
    new_reward_address: string;
    [property: string]: any;
}

export interface UpdatePolicy {
    payer:  string;
    update: CallMessage8;
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
    authorized_updaters: string[];
    /**
     * Default payee policy for users that are not in the balances map.
     */
    default_payee_policy: PayeePolicyClass | PayeePolicyEnum;
    /**
     * A mapping from user address to the policy for that user.
     */
    payees: Array<Array<PayeePolicyClass | string>>;
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

export interface SetPayerForSequencer {
    payer: string;
    [property: string]: any;
}

/**
 * Add a new prover as a bonded prover.
 *
 * Increases the balance of the prover, transferring the funds from the prover account to
 * the rollup.
 */
export interface CallMessage6Class {
    register?: number;
    deposit?:  number;
}

/**
 * Unbonds the prover.
 */
export enum CallMessage6Enum {
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
export interface CallMessage3 {
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
 * Call messages for the test recipient module.
 *
 * Register a route with the given token source and ISM.
 *
 * Update an existing route with new admin or ISM.
 *
 * Add a counterparty router on another chain. This router is trusted. A malicious remote
 * router can steal funds. Each warp route can have at most one remote router for a given
 * destination domain.
 *
 * Remove a counterparty router on another chain.
 *
 * Transfer a token from the local chain to the remote chain.
 */
export interface CallMessageForConfigurableSpec {
    Register?:             RegisterObject;
    Update?:               Update;
    EnrollRemoteRouter?:   EnrollRemoteRouter;
    UnEnrollRemoteRouter?: UnEnrollRemoteRouter;
    TransferRemote?:       TransferRemote;
}

export interface EnrollRemoteRouter {
    /**
     * The domain of the remote chain.
     */
    remote_domain: number;
    /**
     * The router address on the remote chain.
     */
    remote_router_address: string;
    /**
     * The ID of the warp route on the local chain.
     */
    warp_route: string;
    [property: string]: any;
}

export interface RegisterObject {
    /**
     * The authority that can modify the route, if any.
     */
    admin: AdminForConfigurableSpecClass | AdminForConfigurableSpecEnum;
    /**
     * The ISM for this route.
     */
    ism: IsmClass | IsmEnum;
    /**
     * Remote routers to enroll on route registration.
     */
    remote_routers: Array<Array<number | string>>;
    /**
     * The token source for the route.
     */
    token_source: TokenKindClass | TokenKindEnum;
    [property: string]: any;
}

/**
 * Allow the specified address to modify the route. This is extremely insecure, but it seems
 * to be common practice in Hyperlane.
 */
export interface AdminForConfigurableSpecClass {
    InsecureOwner: string;
}

/**
 * No admin - the route is immutable.
 */
export enum AdminForConfigurableSpecEnum {
    None = "None",
}

/**
 * Accepts all messages from a trusted relayer
 *
 * Accepts messages if signed by `threshold` or more of the provided `validators`
 */
export interface IsmClass {
    TrustedRelayer?:    TrustedRelayer;
    MessageIdMultisig?: MessageIDMultisig;
}

export interface MessageIDMultisig {
    /**
     * The number of signatures required to accept a message
     */
    threshold: number;
    /**
     * The addresses of the validators
     */
    validators: string[];
    [property: string]: any;
}

export interface TrustedRelayer {
    /**
     * The address of the trusted relayer, in [`HyperlaneAddress`] format
     */
    relayer: string;
    [property: string]: any;
}

/**
 * Performs no validation. Will accept any message - useful for testing
 */
export enum IsmEnum {
    AlwaysTrust = "AlwaysTrust",
}

/**
 * The token is natively issued on some remote chain, so the local representation is a
 * synthetic token.
 *
 * The token is natively issued on the local chain.
 */
export interface TokenKindClass {
    Synthetic?:  Synthetic;
    Collateral?: Collateral;
}

export interface Collateral {
    /**
     * The ID of the token on the local chain.
     */
    token: string;
    [property: string]: any;
}

export interface Synthetic {
    /**
     * The number of decimal places for the local (synthetic) token.
     *
     * Should be set if remote token should be scaled locally, defaults to remote decimals.
     */
    local_decimals?: number | null;
    /**
     * The number of decimal places of the remote token.
     */
    remote_decimals: number;
    /**
     * The ID of the remote token.
     */
    remote_token_id: string;
    [property: string]: any;
}

/**
 * The token is the native token of the local chain.
 */
export enum TokenKindEnum {
    Native = "Native",
}

export interface TransferRemote {
    /**
     * The amount to transfer.
     */
    amount: number;
    /**
     * The domain of the destination chain.
     */
    destination_domain: number;
    /**
     * A limit for the payment to relayer to cover gas needed for message delivery.
     */
    gas_payment_limit: number;
    /**
     * The recipient on the destination chain.
     */
    recipient: string;
    /**
     * Selected relayer
     */
    relayer?: null | string;
    /**
     * The route to use for the transfer.
     */
    warp_route: string;
    [property: string]: any;
}

export interface UnEnrollRemoteRouter {
    /**
     * The domain of the remote chain.
     */
    remote_domain: number;
    /**
     * The ID of the warp route on the local chain.
     */
    warp_route: string;
    [property: string]: any;
}

export interface Update {
    /**
     * New authority that can modify the route.
     */
    admin?: AdminForConfigurableSpecClass | AdminForConfigurableSpecEnum | null;
    /**
     * New ISM for this route.
     */
    ism?: IsmIsmClass | IsmEnum | null;
    /**
     * The ID of the warp route on the local chain to update.
     */
    warp_route: string;
    [property: string]: any;
}

/**
 * Accepts all messages from a trusted relayer
 *
 * Accepts messages if signed by `threshold` or more of the provided `validators`
 */
export interface IsmIsmClass {
    TrustedRelayer?:    TrustedRelayer;
    MessageIdMultisig?: MessageIDMultisig;
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
        { json: "accounts", js: "accounts", typ: u(undefined, r("CallMessage")) },
        { json: "uniqueness", js: "uniqueness", typ: u(undefined, null) },
        { json: "bank", js: "bank", typ: u(undefined, r("CallMessage2")) },
        { json: "sequencer_registry", js: "sequencer_registry", typ: u(undefined, r("CallMessage3")) },
        { json: "operator_incentives", js: "operator_incentives", typ: u(undefined, r("CallMessage4")) },
        { json: "attester_incentives", js: "attester_incentives", typ: u(undefined, u(r("CallMessage5Class"), r("CallMessage5Enum"))) },
        { json: "prover_incentives", js: "prover_incentives", typ: u(undefined, u(r("CallMessage6Class"), r("CallMessage6Enum"))) },
        { json: "value_setter", js: "value_setter", typ: u(undefined, r("CallMessage7")) },
        { json: "chain_state", js: "chain_state", typ: u(undefined, null) },
        { json: "blob_storage", js: "blob_storage", typ: u(undefined, null) },
        { json: "paymaster", js: "paymaster", typ: u(undefined, r("CallMessage8")) },
        { json: "mailbox", js: "mailbox", typ: u(undefined, r("CallMessage9")) },
        { json: "interchain_gas_paymaster", js: "interchain_gas_paymaster", typ: u(undefined, r("CallMessage10")) },
        { json: "merkle_tree_hook", js: "merkle_tree_hook", typ: u(undefined, null) },
        { json: "warp", js: "warp", typ: u(undefined, r("CallMessageForConfigurableSpec")) },
    ], false),
    "CallMessage": o([
        { json: "insert_credential_id", js: "insert_credential_id", typ: "" },
    ], false),
    "CallMessage5Class": o([
        { json: "register_attester", js: "register_attester", typ: u(undefined, 0) },
        { json: "register_challenger", js: "register_challenger", typ: u(undefined, 0) },
        { json: "deposit_attester", js: "deposit_attester", typ: u(undefined, 0) },
    ], false),
    "CallMessage2": o([
        { json: "create_token", js: "create_token", typ: u(undefined, r("CreateToken")) },
        { json: "transfer", js: "transfer", typ: u(undefined, r("Transfer")) },
        { json: "burn", js: "burn", typ: u(undefined, r("Burn")) },
        { json: "mint", js: "mint", typ: u(undefined, r("Mint")) },
        { json: "freeze", js: "freeze", typ: u(undefined, r("Freeze")) },
    ], false),
    "Burn": o([
        { json: "coins", js: "coins", typ: r("Coins") },
    ], "any"),
    "Coins": o([
        { json: "amount", js: "amount", typ: 0 },
        { json: "token_id", js: "token_id", typ: "" },
    ], "any"),
    "CreateToken": o([
        { json: "admins", js: "admins", typ: a("") },
        { json: "initial_balance", js: "initial_balance", typ: 0 },
        { json: "mint_to_address", js: "mint_to_address", typ: "" },
        { json: "supply_cap", js: "supply_cap", typ: u(undefined, u(0, null)) },
        { json: "token_decimals", js: "token_decimals", typ: u(undefined, u(0, null)) },
        { json: "token_name", js: "token_name", typ: "" },
    ], "any"),
    "Freeze": o([
        { json: "token_id", js: "token_id", typ: "" },
    ], "any"),
    "Mint": o([
        { json: "coins", js: "coins", typ: r("Coins") },
        { json: "mint_to_address", js: "mint_to_address", typ: "" },
    ], "any"),
    "Transfer": o([
        { json: "coins", js: "coins", typ: r("Coins") },
        { json: "to", js: "to", typ: "" },
    ], "any"),
    "CallMessage7": o([
        { json: "set_value", js: "set_value", typ: 0 },
    ], false),
    "CallMessage10": o([
        { json: "set_relayer_config", js: "set_relayer_config", typ: u(undefined, r("SetRelayerConfig")) },
        { json: "update_oracle_data", js: "update_oracle_data", typ: u(undefined, r("UpdateOracleData")) },
        { json: "claim_rewards", js: "claim_rewards", typ: u(undefined, r("ClaimRewards")) },
    ], false),
    "ClaimRewards": o([
        { json: "relayer_address", js: "relayer_address", typ: "" },
    ], "any"),
    "SetRelayerConfig": o([
        { json: "beneficiary", js: "beneficiary", typ: u(undefined, u(null, "")) },
        { json: "default_gas", js: "default_gas", typ: 0 },
        { json: "domain_default_gas", js: "domain_default_gas", typ: a(r("DomainDefaultGas")) },
        { json: "domain_oracle_data", js: "domain_oracle_data", typ: a(r("DomainOracleData")) },
    ], "any"),
    "DomainDefaultGas": o([
        { json: "default_gas", js: "default_gas", typ: 0 },
        { json: "domain", js: "domain", typ: 0 },
    ], "any"),
    "DomainOracleData": o([
        { json: "data_value", js: "data_value", typ: r("ExchangeRateAndGasPrice") },
        { json: "domain", js: "domain", typ: 0 },
    ], "any"),
    "ExchangeRateAndGasPrice": o([
        { json: "gas_price", js: "gas_price", typ: 0 },
        { json: "token_exchange_rate", js: "token_exchange_rate", typ: 0 },
    ], "any"),
    "UpdateOracleData": o([
        { json: "domain", js: "domain", typ: 0 },
        { json: "oracle_data", js: "oracle_data", typ: r("ExchangeRateAndGasPrice") },
    ], "any"),
    "CallMessage9": o([
        { json: "dispatch", js: "dispatch", typ: u(undefined, r("Dispatch")) },
        { json: "process", js: "process", typ: u(undefined, r("Process")) },
        { json: "announce", js: "announce", typ: u(undefined, r("Announce")) },
    ], false),
    "Announce": o([
        { json: "signature", js: "signature", typ: "" },
        { json: "storage_location", js: "storage_location", typ: "" },
        { json: "validator_address", js: "validator_address", typ: "" },
    ], "any"),
    "Dispatch": o([
        { json: "body", js: "body", typ: "" },
        { json: "domain", js: "domain", typ: 0 },
        { json: "gas_payment_limit", js: "gas_payment_limit", typ: 0 },
        { json: "metadata", js: "metadata", typ: u(undefined, u(null, "")) },
        { json: "recipient", js: "recipient", typ: "" },
        { json: "relayer", js: "relayer", typ: u(undefined, u(null, "")) },
    ], "any"),
    "Process": o([
        { json: "message", js: "message", typ: "" },
        { json: "metadata", js: "metadata", typ: "" },
    ], "any"),
    "CallMessage4": o([
        { json: "update_reward_address", js: "update_reward_address", typ: r("UpdateRewardAddress") },
    ], false),
    "UpdateRewardAddress": o([
        { json: "new_reward_address", js: "new_reward_address", typ: "" },
    ], "any"),
    "UpdatePolicy": o([
        { json: "payer", js: "payer", typ: "" },
        { json: "update", js: "update", typ: r("CallMessage8") },
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
        { json: "authorized_updaters", js: "authorized_updaters", typ: a("") },
        { json: "default_payee_policy", js: "default_payee_policy", typ: u(r("PayeePolicyClass"), r("PayeePolicyEnum")) },
        { json: "payees", js: "payees", typ: a(a(u(r("PayeePolicyClass"), ""))) },
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
    "SetPayerForSequencer": o([
        { json: "payer", js: "payer", typ: "" },
    ], "any"),
    "CallMessage6Class": o([
        { json: "register", js: "register", typ: u(undefined, 0) },
        { json: "deposit", js: "deposit", typ: u(undefined, 0) },
    ], false),
    "CallMessage3": o([
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
    "CallMessageForConfigurableSpec": o([
        { json: "Register", js: "Register", typ: u(undefined, r("RegisterObject")) },
        { json: "Update", js: "Update", typ: u(undefined, r("Update")) },
        { json: "EnrollRemoteRouter", js: "EnrollRemoteRouter", typ: u(undefined, r("EnrollRemoteRouter")) },
        { json: "UnEnrollRemoteRouter", js: "UnEnrollRemoteRouter", typ: u(undefined, r("UnEnrollRemoteRouter")) },
        { json: "TransferRemote", js: "TransferRemote", typ: u(undefined, r("TransferRemote")) },
    ], false),
    "EnrollRemoteRouter": o([
        { json: "remote_domain", js: "remote_domain", typ: 0 },
        { json: "remote_router_address", js: "remote_router_address", typ: "" },
        { json: "warp_route", js: "warp_route", typ: "" },
    ], "any"),
    "RegisterObject": o([
        { json: "admin", js: "admin", typ: u(r("AdminForConfigurableSpecClass"), r("AdminForConfigurableSpecEnum")) },
        { json: "ism", js: "ism", typ: u(r("IsmClass"), r("IsmEnum")) },
        { json: "remote_routers", js: "remote_routers", typ: a(a(u(0, ""))) },
        { json: "token_source", js: "token_source", typ: u(r("TokenKindClass"), r("TokenKindEnum")) },
    ], "any"),
    "AdminForConfigurableSpecClass": o([
        { json: "InsecureOwner", js: "InsecureOwner", typ: "" },
    ], false),
    "IsmClass": o([
        { json: "TrustedRelayer", js: "TrustedRelayer", typ: u(undefined, r("TrustedRelayer")) },
        { json: "MessageIdMultisig", js: "MessageIdMultisig", typ: u(undefined, r("MessageIDMultisig")) },
    ], false),
    "MessageIDMultisig": o([
        { json: "threshold", js: "threshold", typ: 0 },
        { json: "validators", js: "validators", typ: a("") },
    ], "any"),
    "TrustedRelayer": o([
        { json: "relayer", js: "relayer", typ: "" },
    ], "any"),
    "TokenKindClass": o([
        { json: "Synthetic", js: "Synthetic", typ: u(undefined, r("Synthetic")) },
        { json: "Collateral", js: "Collateral", typ: u(undefined, r("Collateral")) },
    ], false),
    "Collateral": o([
        { json: "token", js: "token", typ: "" },
    ], "any"),
    "Synthetic": o([
        { json: "local_decimals", js: "local_decimals", typ: u(undefined, u(0, null)) },
        { json: "remote_decimals", js: "remote_decimals", typ: 0 },
        { json: "remote_token_id", js: "remote_token_id", typ: "" },
    ], "any"),
    "TransferRemote": o([
        { json: "amount", js: "amount", typ: 0 },
        { json: "destination_domain", js: "destination_domain", typ: 0 },
        { json: "gas_payment_limit", js: "gas_payment_limit", typ: 0 },
        { json: "recipient", js: "recipient", typ: "" },
        { json: "relayer", js: "relayer", typ: u(undefined, u(null, "")) },
        { json: "warp_route", js: "warp_route", typ: "" },
    ], "any"),
    "UnEnrollRemoteRouter": o([
        { json: "remote_domain", js: "remote_domain", typ: 0 },
        { json: "warp_route", js: "warp_route", typ: "" },
    ], "any"),
    "Update": o([
        { json: "admin", js: "admin", typ: u(undefined, u(r("AdminForConfigurableSpecClass"), r("AdminForConfigurableSpecEnum"), null)) },
        { json: "ism", js: "ism", typ: u(undefined, u(r("IsmIsmClass"), r("IsmEnum"), null)) },
        { json: "warp_route", js: "warp_route", typ: "" },
    ], "any"),
    "IsmIsmClass": o([
        { json: "TrustedRelayer", js: "TrustedRelayer", typ: u(undefined, r("TrustedRelayer")) },
        { json: "MessageIdMultisig", js: "MessageIdMultisig", typ: u(undefined, r("MessageIDMultisig")) },
    ], false),
    "CallMessage5Enum": [
        "begin_exit_attester",
        "exit_attester",
        "exit_challenger",
    ],
    "AuthorizedSequencersEnum": [
        "all",
    ],
    "PayeePolicyEnum": [
        "deny",
    ],
    "CallMessage6Enum": [
        "exit",
    ],
    "AdminForConfigurableSpecEnum": [
        "None",
    ],
    "IsmEnum": [
        "AlwaysTrust",
    ],
    "TokenKindEnum": [
        "Native",
    ],
};
