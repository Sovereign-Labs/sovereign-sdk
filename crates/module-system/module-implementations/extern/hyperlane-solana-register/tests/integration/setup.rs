use std::str::FromStr;
use std::sync::Arc;

use sov_bank::Amount;
use sov_hyperlane_integration::warp::{Admin, TokenKind};
use sov_hyperlane_integration::{
    HyperlaneAddress, InterchainGasPaymaster, Ism, Mailbox as RawMailbox, MerkleTreeHook, Message,
    Warp, WarpCallMessage, WarpEvent,
};
use sov_hyperlane_register_module::{SolanaDeployment, SolanaRegistration};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::macros::config_value;
use sov_modules_api::{
    configurable_spec::ConfigurableSpec, Base58Address, HexHash, HexString, SafeVec,
};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    generate_runtime, AsUser, MockDaSpec, MockZkvm, TestUser, TransactionTestCase,
};

pub type Mailbox<S> = RawMailbox<S, SolanaRegistration<S>>;
pub type S = ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, Base58Address, Native>;
pub type RT = TestRuntime<S>;
type WarpRouteId = HexHash;

pub const SOLANA_PROGRAM_ID: &str = "692KZJaoe2KRcD6uhCQDLLXnLNA5ZLnfvdqjE4aX9iu1";
pub const SOLANA_HYPERLANE_DOMAIN_ID: u32 = 1337;

generate_runtime! {
    name: TestRuntime,
    modules: [mailbox: Mailbox<S>, warp: Warp<S>, merkle_tree_hooks: MerkleTreeHook<S>, interchain_gas_paymaster: InterchainGasPaymaster<S>, solana_register: SolanaRegistration<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::zk::config::MinimalZkGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: HyperlaneAddress],
    kernel_type: sov_test_utils::runtime::BasicKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestRuntime<S>>,
    auth_call_wrapper: |call| call,
}

pub fn generate_with_additional_accounts(num_accounts: usize) -> HighLevelZkGenesisConfig<S> {
    HighLevelZkGenesisConfig::generate_with_additional_accounts_and_code_commitments(
        num_accounts,
        Default::default(),
        Default::default(),
    )
}

pub struct SetupParams {
    pub runner: TestRunner<RT, S>,
    pub admin: TestUser<S>,
    pub user: TestUser<S>,
    pub module_admin: TestUser<S>,
}

pub fn setup() -> SetupParams {
    let genesis_config = generate_with_additional_accounts(4);

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_account = genesis_config.additional_accounts()[1].clone();
    let module_admin = genesis_config.additional_accounts()[2].clone();
    let registration_conf = sov_hyperlane_register_module::GenesisConfig {
        admin: module_admin.address(),
        deployment: Some(SolanaDeployment {
            domain_id: SOLANA_HYPERLANE_DOMAIN_ID,
            program_id: Base58Address::from_str(SOLANA_PROGRAM_ID).unwrap(),
        }),
        ism: Some(Ism::AlwaysTrust),
    };

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.clone().into(),
        (),
        (),
        (),
        (),
        registration_conf,
    );

    SetupParams {
        runner: TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default()),
        admin: admin_account,
        user: extra_account,
        module_admin,
    }
}

pub fn register_basic_warp_route(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
) -> WarpRouteId {
    register_warp_route_with_ism_and_token_source(runner, user, Ism::AlwaysTrust, TokenKind::Native)
}

pub fn register_warp_route_with_ism_and_token_source(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    ism: Ism,
    token_source: TokenKind,
) -> WarpRouteId {
    // The borrow checker doesn't know that the closure runs before the end of execute transaction, so it complains about lifetimes
    // if we don't Arc the warp route id
    let warp_route_id = Arc::new(std::sync::Mutex::new(HexString([0; 32])));
    let id_ref = warp_route_id.clone();
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Warp<S>>(WarpCallMessage::Register {
            admin: Admin::InsecureOwner(user.address()),
            token_source,
            ism,
            remote_routers: SafeVec::new(),
            inbound_transferrable_tokens_limit: Amount::MAX,
            inbound_limit_replenishment_per_slot: Amount::MAX,
            outbound_transferrable_tokens_limit: Amount::MAX,
            outbound_limit_replenishment_per_slot: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Recipient was not registered successfully"
            );
            for event in result.events {
                if let TestRuntimeEvent::Warp(WarpEvent::RouteRegistered { route_id, .. }) = event {
                    *id_ref.lock().unwrap() = route_id;
                }
            }
        }),
    });
    let id = *warp_route_id.lock().unwrap();
    assert!(id != HexString([0; 32]), "Warp route was not registered");
    id
}

pub fn make_message(
    nonce: u32,
    origin: u32,
    sender: HexHash,
    destination: u32,
    recipient: HexHash,
    body: HexString,
) -> Message {
    Message {
        version: 3,
        nonce,
        origin_domain: origin,
        sender,
        dest_domain: destination,
        recipient,
        body,
    }
}

pub fn make_invalid_message(nonce: u32, recipient: HexHash, body: HexString) -> Message {
    let program_b58 = Base58Address::from_str(SOLANA_PROGRAM_ID).unwrap();
    let program_id = HexHash::new(program_b58.0);

    make_message(
        nonce,
        0, // wrong origin domain
        program_id,
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient,
        body,
    )
}

pub fn make_valid_message(nonce: u32, recipient: HexHash, body: HexString) -> Message {
    let program_b58 = Base58Address::from_str(SOLANA_PROGRAM_ID).unwrap();
    let program_id = HexHash::new(program_b58.0);

    make_message(
        nonce,
        SOLANA_HYPERLANE_DOMAIN_ID,
        program_id,
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient,
        body,
    )
}
