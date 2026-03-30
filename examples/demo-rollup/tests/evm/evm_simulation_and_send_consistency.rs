use crate::evm::evm_test_helper::{
    call_all_endpoints, create_simple_storage_client, setup_test_rollup,
    setup_test_rollup_with_selective_paymaster, tx_count, EndpointResults,
    AFFORDABILITY_SIGNER_PRIV_KEY, EVM_EXTENSION, INSUFFICIENT_FUNDS_ERROR, MAX_FEE_PER_GAS,
    PAYMASTER_SIGNER_PRIV_KEY, SENDER_PRIV_KEY,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, TxKind, U256, U64};
use alloy_rpc_types_eth::{AccessListResult, TransactionRequest};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::rpc_params;
use jsonrpsee::types::ErrorObjectOwned;
use sov_eth_client::SimpleStorageClient;

// Rollup thresholds (from genesis config):
//   base_fee        = 10 wei
//   sender_balance  = 100_000_000_000_000_000 (1e17)
//   tx_gas_limit    = 30_000_000
//   intrinsic_gas   = 21_000 (simple ETH transfer)
const BASE_FEE: u128 = 10;

// ---------------------------------------------------------------------------
// Test input types
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TestTransactionRequest {
    max_fee_per_gas: Option<u128>,
    max_priority_fee_per_gas: Option<u128>,
    gas: Option<u64>,
    value: Option<U256>,
    to: Option<Address>,
}

impl TestTransactionRequest {
    fn build_tx_request(&self, sender: Address) -> TransactionRequest {
        TransactionRequest {
            from: Some(sender),
            to: self.to.map(TxKind::Call),
            max_fee_per_gas: self.max_fee_per_gas,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas,
            gas: self.gas,
            value: self.value,
            ..Default::default()
        }
    }
}

/// Generates all transaction field combinations from the boundary-value sets.
fn all_tx_field_combinations() -> Vec<TestTransactionRequest> {
    let max_fees: &[Option<u128>] = &[
        None,
        Some(0),
        Some(9),
        Some(10),
        Some(11),
        Some(10_000_000_000_000),
    ];
    let gases: &[Option<u64>] = &[None, Some(0), Some(20_999), Some(21_000), Some(30_000_001)];
    let values: &[Option<U256>] = &[
        None,
        Some(U256::ZERO),
        Some(U256::from(1u64)),
        Some(U256::from(100_000_000_000_000_001u128)),
    ];
    let tos: &[Option<Address>] = &[None, Some(Address::repeat_byte(0x22))];

    let mut combos = Vec::new();
    for &max_fee_per_gas in max_fees {
        // Priority fee options depend on whether max_fee is set (mirrors original Arbitrary impl)
        let priority_fees: &[Option<u128>] = if max_fee_per_gas.is_none() {
            &[None, Some(0)]
        } else {
            &[None, Some(0), Some(11)]
        };
        for &max_priority_fee_per_gas in priority_fees {
            for &gas in gases {
                for value in values {
                    for to in tos {
                        combos.push(TestTransactionRequest {
                            max_fee_per_gas,
                            max_priority_fee_per_gas,
                            gas,
                            value: *value,
                            to: *to,
                        });
                    }
                }
            }
        }
    }
    combos
}

#[derive(Debug, Clone, Copy)]
enum NonceOption {
    Below,
    Match,
    Future,
}

#[derive(Debug, Clone, Copy)]
enum RegularTestAccount {
    Funded,
    UnfundedUncovered,
}

impl RegularTestAccount {
    fn priv_key(self) -> &'static str {
        match self {
            Self::Funded => SENDER_PRIV_KEY,
            Self::UnfundedUncovered => AFFORDABILITY_SIGNER_PRIV_KEY,
        }
    }

    fn is_funded(self) -> bool {
        matches!(self, Self::Funded)
    }
}

#[derive(Debug, Clone, Copy)]
enum PaymasterTestAccount {
    FundedCovered,
    UnfundedCovered,
    UnfundedUncovered,
}

impl PaymasterTestAccount {
    fn priv_key(self) -> &'static str {
        match self {
            Self::FundedCovered => SENDER_PRIV_KEY,
            Self::UnfundedCovered => PAYMASTER_SIGNER_PRIV_KEY,
            Self::UnfundedUncovered => AFFORDABILITY_SIGNER_PRIV_KEY,
        }
    }

    fn is_funded(self) -> bool {
        matches!(self, Self::FundedCovered)
    }
}

// ---------------------------------------------------------------------------
// Nonce setup helper
// ---------------------------------------------------------------------------

/// Applies the nonce option to the request. For `Below`, bumps the account nonce
/// by sending a self-transfer (only for funded accounts; unfunded falls back to `Match`).
async fn apply_nonce(
    nonce_option: NonceOption,
    is_funded: bool,
    ws_client: &SimpleStorageClient,
    signer: &PrivateKeySigner,
    request: &mut TransactionRequest,
) -> anyhow::Result<()> {
    match nonce_option {
        NonceOption::Below if is_funded => {
            // Sync client nonce with on-chain nonce; the main loop sends raw
            // transactions via call_all_endpoints which bypasses the client's
            // internal nonce counter.
            let on_chain_nonce = tx_count(ws_client, signer.address(), "latest").await?;
            ws_client
                .nonce
                .store(on_chain_nonce, std::sync::atomic::Ordering::SeqCst);
            let hash = ws_client.send_eth(signer.address(), U256::ZERO).await;
            ws_client.wait_for_receipt(hash).await;
            request.nonce = Some(0);
        }
        NonceOption::Future => {
            let nonce = tx_count(ws_client, signer.address(), "latest").await?;
            request.nonce = Some(nonce + 1);
        }
        // Match, or Below for unfunded accounts (nonce is already 0, nothing below it)
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Consistency check
// ---------------------------------------------------------------------------

/// Returns true when createAccessList is expected to agree with estimate+send.
///
/// createAccessList diverges from estimate/send in three known ways:
///   - It skips fee cap validation (accepts below-base-fee requests)
///   - It skips nonce validation
///   - It skips affordability checks (gas_price hardcoded to 0)
///
/// So strict checking is safe only when none of these divergences apply.
fn should_strict_check(
    nonce_option: NonceOption,
    is_funded: bool,
    max_fee_per_gas: Option<u128>,
) -> bool {
    matches!(nonce_option, NonceOption::Match)
        && is_funded
        && max_fee_per_gas.is_some_and(|f| f >= BASE_FEE)
}

fn check_consistency(
    results: &EndpointResults,
    explicit_gas: Option<u64>,
    explicit_max_fee: Option<u128>,
    nonce_option: NonceOption,
    strict_check: bool,
    context: &str,
) {
    let EndpointResults {
        estimate_gas,
        call,
        create_access_list,
        send_raw_tx,
    } = results;

    // ── Step 1: estimateGas vs sendRawTransaction must agree ──
    //
    // Known expected divergences where estimate=Ok but send=Err:
    //   (a) user provided explicit gas below the estimated gas (user error)
    //   (b) simulation doesn't validate nonce; Below/Future nonce causes send to fail
    //   (c) fee fields omitted → estimate skips affordability preflight,
    //       but raw tx gets base fee filled in and sequencer checks affordability
    match (estimate_gas, send_raw_tx) {
        (Ok(_), Ok(_)) | (Err(_), Err(_)) => {
            // Primary pair agrees — checked further in step 2.
        }
        (Ok(estimated), Err(_)) if explicit_gas.is_some_and(|g| g < estimated.to::<u64>()) => {
            // (a) explicit gas cap below estimate
        }
        (Ok(_), Err(_)) if !matches!(nonce_option, NonceOption::Match) => {
            // (b) simulation skips nonce check
        }
        (Ok(_), Err(send_err))
            if explicit_max_fee.is_none()
                && send_err.to_string().contains(INSUFFICIENT_FUNDS_ERROR) =>
        {
            // (c) affordability preflight skipped without fee fields
        }
        _ => {
            panic!(
                "estimateGas and sendRawTransaction disagree:\n\
                 {context}\n\
                 estimateGas={estimate_gas:?}\n\
                 sendRawTransaction={send_raw_tx:?}"
            );
        }
    }

    // ── Step 2: simulation endpoints (call, createAccessList) ──
    //
    // eth_call and createAccessList skip affordability and nonce checks,
    // so they can succeed when estimate/send fail. When strict_check is
    // true (funded account, valid fee, matching nonce), they must agree
    // with estimate.
    if strict_check {
        if estimate_gas.is_ok() {
            assert!(
                call.is_ok(),
                "strict: eth_call should succeed when estimateGas succeeds:\n\
                 {context}\n\
                 estimateGas={estimate_gas:?}\n\
                 eth_call={call:?}"
            );
            assert!(
                create_access_list.is_ok(),
                "strict: createAccessList should succeed when estimateGas succeeds:\n\
                 {context}\n\
                 estimateGas={estimate_gas:?}\n\
                 createAccessList={create_access_list:?}"
            );
        }
        if estimate_gas.is_err() && explicit_gas.is_none() {
            // When estimate fails under strict conditions, call must also fail:
            // same fee validation applies to both.
            //
            // Exception: when explicit gas is set, estimateGas may fail with gas
            // exhaustion (STF pipeline gas metering) while eth_call succeeds
            // (direct EVM simulation with different gas handling).
            assert!(
                call.is_err(),
                "strict: eth_call should fail when estimateGas fails:\n\
                 {context}\n\
                 estimateGas={estimate_gas:?}\n\
                 eth_call={call:?}"
            );
            // createAccessList intentionally skips fee cap validation,
            // so it may succeed even when estimate fails. No assertion here.
        }
    }
}

// ---------------------------------------------------------------------------
// Inner test logic (shared rollup variant)
// ---------------------------------------------------------------------------

async fn run_consistency_check(
    ws_client: &SimpleStorageClient,
    signer: &PrivateKeySigner,
    request: &TestTransactionRequest,
    nonce_option: NonceOption,
    is_funded: bool,
    account_label: &str,
) -> anyhow::Result<()> {
    let mut tx_request = request.build_tx_request(signer.address());
    apply_nonce(nonce_option, is_funded, ws_client, signer, &mut tx_request).await?;

    let strict_check = should_strict_check(nonce_option, is_funded, request.max_fee_per_gas);
    let context = format!(
        "account={account_label} nonce={nonce_option:?} request={request:?} nonce_field={:?}",
        tx_request.nonce
    );
    let results = call_all_endpoints(ws_client, &tx_request, signer).await;
    check_consistency(
        &results,
        tx_request.gas,
        tx_request.max_fee_per_gas,
        nonce_option,
        strict_check,
        &context,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Smoke test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn smoke_test_evm_endpoint_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let signer: PrivateKeySigner = SENDER_PRIV_KEY.parse()?;

    let request = TestTransactionRequest {
        max_fee_per_gas: Some(10),
        max_priority_fee_per_gas: Some(0),
        gas: None,
        value: Some(U256::from(1u64)),
        to: Some(Address::repeat_byte(0x22)),
    };

    run_consistency_check(
        &ws_client,
        &signer,
        &request,
        NonceOption::Match,
        true,
        "Funded",
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "estimateGas and sendRawTransaction disagree")]
async fn explicit_gas_matching_estimate_does_not_whitelist_unrelated_send_failure() {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, SENDER_PRIV_KEY).await;
    let recipient = Address::repeat_byte(0x27);

    let estimate_request = TransactionRequest {
        from: Some(ws_client.address()),
        to: Some(TxKind::Call(recipient)),
        value: Some(U256::ZERO),
        max_fee_per_gas: Some(MAX_FEE_PER_GAS),
        max_priority_fee_per_gas: Some(0),
        ..Default::default()
    };
    let estimate: U64 = ws_client
        .ws
        .request("eth_estimateGas", rpc_params![&estimate_request, "latest"])
        .await
        .expect("estimate should succeed so the test can derive the gas value");
    let estimate = estimate.to::<u64>();

    // Fabricate results where estimate=Ok but send fails for a reason unrelated
    // to gas (fee validation error). check_consistency must catch this as a true
    // disagreement rather than whitelisting it via the gas-shortfall arm.
    let results = EndpointResults {
        estimate_gas: Ok(U64::from(estimate)),
        call: Ok("0x".to_string()),
        create_access_list: Ok(AccessListResult {
            access_list: Default::default(),
            gas_used: U256::from(estimate),
            error: None,
        }),
        send_raw_tx: Err(jsonrpsee::core::client::Error::Call(
            ErrorObjectOwned::owned(
                -32003,
                "max priority fee per gas higher than max fee per gas",
                None::<()>,
            ),
        )),
    };

    check_consistency(
        &results,
        Some(estimate),
        Some(MAX_FEE_PER_GAS),
        NonceOption::Match,
        false,
        "estimate-matching gas with unrelated send error",
    );
}

// ---------------------------------------------------------------------------
// Exhaustive enumeration tests
// ---------------------------------------------------------------------------

/// Representative subset of tx combinations for nonce variant testing.
/// Avoids the full Cartesian product since Below nonce requires a self-transfer
/// (1 block time wait per case).
fn representative_combos() -> Vec<TestTransactionRequest> {
    vec![
        // Funded happy path
        TestTransactionRequest {
            max_fee_per_gas: Some(11),
            max_priority_fee_per_gas: Some(0),
            gas: None,
            value: Some(U256::from(1u64)),
            to: Some(Address::repeat_byte(0x22)),
        },
        // No fee fields (affordability preflight skipped)
        TestTransactionRequest {
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            gas: None,
            value: Some(U256::ZERO),
            to: Some(Address::repeat_byte(0x22)),
        },
        // Below base fee
        TestTransactionRequest {
            max_fee_per_gas: Some(9),
            max_priority_fee_per_gas: Some(0),
            gas: Some(21_000),
            value: Some(U256::ZERO),
            to: Some(Address::repeat_byte(0x22)),
        },
        // Contract creation
        TestTransactionRequest {
            max_fee_per_gas: Some(10),
            max_priority_fee_per_gas: None,
            gas: None,
            value: None,
            to: None,
        },
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn exhaustive_regular_rollup_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let combos = all_tx_field_combinations();
    let accounts = [
        RegularTestAccount::Funded,
        RegularTestAccount::UnfundedUncovered,
    ];

    for account in accounts {
        let ws_client = create_simple_storage_client(rollup.http_addr, account.priv_key()).await;
        let signer: PrivateKeySigner = account.priv_key().parse()?;

        // Main loop: Match nonce across all tx field combinations
        for (i, combo) in combos.iter().enumerate() {
            run_consistency_check(
                &ws_client,
                &signer,
                combo,
                NonceOption::Match,
                account.is_funded(),
                &format!("{account:?}"),
            )
            .await
            .unwrap_or_else(|e| panic!("case {i} failed for {account:?}: {e}"));
        }

        // Future nonce: representative subset (no extra tx overhead)
        for combo in &representative_combos() {
            run_consistency_check(
                &ws_client,
                &signer,
                combo,
                NonceOption::Future,
                account.is_funded(),
                &format!("{account:?}"),
            )
            .await?;
        }

        // Below nonce: representative subset (sends a self-transfer per case, only for funded)
        if account.is_funded() {
            for combo in &representative_combos() {
                run_consistency_check(
                    &ws_client,
                    &signer,
                    combo,
                    NonceOption::Below,
                    account.is_funded(),
                    &format!("{account:?}"),
                )
                .await?;
            }
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn exhaustive_paymaster_rollup_consistency() -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_selective_paymaster(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let combos = all_tx_field_combinations();
    let accounts = [
        PaymasterTestAccount::FundedCovered,
        PaymasterTestAccount::UnfundedCovered,
        PaymasterTestAccount::UnfundedUncovered,
    ];

    for account in accounts {
        let ws_client = create_simple_storage_client(rollup.http_addr, account.priv_key()).await;
        let signer: PrivateKeySigner = account.priv_key().parse()?;

        // Main loop: Match nonce across all tx field combinations
        for (i, combo) in combos.iter().enumerate() {
            run_consistency_check(
                &ws_client,
                &signer,
                combo,
                NonceOption::Match,
                account.is_funded(),
                &format!("{account:?}"),
            )
            .await
            .unwrap_or_else(|e| panic!("case {i} failed for {account:?}: {e}"));
        }

        // Future nonce: representative subset
        for combo in &representative_combos() {
            run_consistency_check(
                &ws_client,
                &signer,
                combo,
                NonceOption::Future,
                account.is_funded(),
                &format!("{account:?}"),
            )
            .await?;
        }

        // Below nonce: representative subset (only for funded accounts)
        if account.is_funded() {
            for combo in &representative_combos() {
                run_consistency_check(
                    &ws_client,
                    &signer,
                    combo,
                    NonceOption::Below,
                    account.is_funded(),
                    &format!("{account:?}"),
                )
                .await?;
            }
        }
    }

    Ok(())
}
