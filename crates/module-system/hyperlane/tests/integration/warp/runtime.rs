use std::collections::HashMap;
use std::sync::Arc;

use sov_bank::Amount;
use sov_hyperlane_integration::igp::ExchangeRateAndGasPrice;
use sov_hyperlane_integration::warp::{Admin, TokenKind};
use sov_hyperlane_integration::{
    HyperlaneAddress, InterchainGasPaymaster, InterchainGasPaymasterCallMessage, Ism,
    Mailbox as RawMailbox, MerkleTreeHook, Warp, WarpCallMessage, WarpEvent,
};
use sov_modules_api::{HexHash, HexString, SafeVec, Spec};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{generate_runtime, AsUser, TestSpec, TestUser, TransactionTestCase};

use crate::igp::{default_gas_hashmap_to_safe_vec, oracle_data_hashmap_to_safe_vec};

pub type Mailbox<S> = RawMailbox<S, Warp<S>>;
pub type S = TestSpec;
pub type RT = TestRuntime<S>;
type WarpRouteId = HexHash;

generate_runtime! {
    name: TestRuntime,
    modules: [mailbox: Mailbox<S>, warp: Warp<S>, merkle_tree_hooks: MerkleTreeHook<S>, interchain_gas_paymaster: InterchainGasPaymaster<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::zk::config::MinimalZkGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: HyperlaneAddress],
    kernel_type: sov_test_utils::runtime::BasicKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestRuntime<S>>,
    auth_call_wrapper: |call| call,
}
/// The input for the runtime's authenticator functionality.
#[derive(std::fmt::Debug, Clone, borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct AuthenticatorInput(sov_modules_api::RawTx);

pub const CONFIGURED_DOMAIN: u32 = 1;
pub const CONFIGURED_REMOTE_ROUTER_ADDRESS: HexHash = HexString([1; 32]);

#[allow(clippy::type_complexity)]
pub fn setup() -> (
    TestRunner<TestRuntime<S>, S>,
    TestUser<S>,
    TestUser<S>,
    TestUser<S>,
) {
    let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(3);

    let admin_account = genesis_config.additional_accounts()[0].clone();
    let extra_account = genesis_config.additional_accounts()[1].clone();
    let relayer_account = genesis_config.additional_accounts()[1].clone();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.clone().into(), (), (), (), ());

    (
        TestRunner::new_with_genesis(genesis.into_genesis_params(), Default::default()),
        admin_account,
        extra_account,
        relayer_account,
    )
}

pub fn register_basic_warp_route(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
) -> WarpRouteId {
    register_warp_route_with_ism_and_token_source(runner, user, Ism::AlwaysTrust, TokenKind::Native)
}

pub fn register_basic_warp_route_and_enroll_router(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
) -> WarpRouteId {
    register_basic_warp_route_and_enroll_router_with_ism(runner, user, Ism::AlwaysTrust)
}

pub fn enroll_router(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    warp_route_id: WarpRouteId,
) {
    runner.execute_transaction(TransactionTestCase {
        input: user.create_plain_message::<RT, Warp<S>>(WarpCallMessage::EnrollRemoteRouter {
            warp_route: warp_route_id,
            remote_domain: CONFIGURED_DOMAIN,
            remote_router_address: CONFIGURED_REMOTE_ROUTER_ADDRESS,
        }),
        assert: Box::new(move |result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "Enrollment transaction should be successful"
            );
        }),
    });
}

pub fn register_basic_warp_route_and_enroll_router_with_rate_limiter(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    transferrable_tokens_limit: Amount,
    limit_replenishment_per_slot: Amount,
) -> WarpRouteId {
    let warp_route_id = register_warp_route_with_ism_and_token_source_and_rate_limiter(
        runner,
        user,
        Ism::AlwaysTrust,
        TokenKind::Native,
        transferrable_tokens_limit,
        limit_replenishment_per_slot,
    );
    enroll_router(runner, user, warp_route_id);
    warp_route_id
}

pub fn register_basic_warp_route_and_enroll_router_with_ism(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    ism: Ism,
) -> WarpRouteId {
    let warp_route_id = register_warp_route_with_ism_and_token_source_and_rate_limiter(
        runner,
        user,
        ism,
        TokenKind::Native,
        Amount::MAX,
        Amount::MAX,
    );
    enroll_router(runner, user, warp_route_id);
    warp_route_id
}

pub fn register_warp_route_with_ism_and_token_source(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    ism: Ism,
    token_source: TokenKind,
) -> WarpRouteId {
    register_warp_route_with_ism_and_token_source_and_rate_limiter(
        runner,
        user,
        ism,
        token_source,
        Amount::MAX,
        Amount::MAX,
    )
}

pub fn register_warp_route_with_ism_and_token_source_and_rate_limiter(
    runner: &mut TestRunner<RT, S>,
    user: &TestUser<S>,
    ism: Ism,
    token_source: TokenKind,
    transferrable_tokens_limit: Amount,
    limit_replenishment_per_slot: Amount,
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
            inbound_transferrable_tokens_limit: transferrable_tokens_limit,
            inbound_limit_replenishment_per_slot: limit_replenishment_per_slot,
            outbound_transferrable_tokens_limit: transferrable_tokens_limit,
            outbound_limit_replenishment_per_slot: limit_replenishment_per_slot,
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

pub fn register_relayer_with_dummy_igp(
    runner: &mut TestRunner<RT, S>,
    relayer: &TestUser<S>,
    domain: u32,
) {
    let domain_oracle_data = HashMap::from([(
        domain,
        ExchangeRateAndGasPrice {
            gas_price: Amount(100),
            token_exchange_rate: 100,
        },
    )]);
    let domain_default_gas = HashMap::from([(domain, Amount(100))]);
    register_relayer_with_igp(
        runner,
        relayer,
        domain_oracle_data,
        domain_default_gas,
        Amount(100),
        None,
    );
}

pub fn register_relayer_with_igp(
    runner: &mut TestRunner<RT, S>,
    relayer: &TestUser<S>,
    domain_oracle_data: HashMap<u32, ExchangeRateAndGasPrice>,
    domain_default_gas: HashMap<u32, Amount>,
    default_gas: Amount,
    beneficiary: Option<<S as Spec>::Address>,
) {
    runner.execute_transaction(TransactionTestCase {
        input: relayer.create_plain_message::<RT, InterchainGasPaymaster<S>>(
            InterchainGasPaymasterCallMessage::SetRelayerConfig {
                domain_oracle_data: oracle_data_hashmap_to_safe_vec(domain_oracle_data.clone()),
                domain_default_gas: default_gas_hashmap_to_safe_vec(domain_default_gas.clone()),
                default_gas,
                beneficiary,
            },
        ),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_successful(),
                "IGP set relayer config was not done successfully"
            );
        }),
    });
}
