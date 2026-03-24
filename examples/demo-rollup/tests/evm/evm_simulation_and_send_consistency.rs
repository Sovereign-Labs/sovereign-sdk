use crate::evm::evm_test_helper::{
    call_all_endpoints, create_simple_storage_client, setup_test_rollup,
    setup_test_rollup_with_selective_paymaster, tx_count, EndpointResults,
    AFFORDABILITY_SIGNER_PRIV_KEY, EVM_EXTENSION, INSUFFICIENT_FUNDS_ERROR,
    PAYMASTER_SIGNER_PRIV_KEY, SENDER_PRIV_KEY,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, TxKind, U256, U64};
use alloy_rpc_types_eth::{AccessListResult, TransactionRequest};
use arbitrary::Arbitrary;
use arbitrary::Unstructured;
use jsonrpsee::types::ErrorObjectOwned;
use proptest::prelude::*;
use sov_eth_client::SimpleStorageClient;

// ---------------------------------------------------------------------------
// Test input types
// ---------------------------------------------------------------------------

pub struct TestTransactionRequest {
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub gas: Option<u64>,
    pub value: Option<U256>,
    pub to: Option<Address>,
}

impl TestTransactionRequest {
    pub fn build_tx_request(self, sender: Address) -> TransactionRequest {
        let TestTransactionRequest {
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas,
            value,
            to,
        } = self;
        TransactionRequest {
            from: Some(sender),
            to: to.map(TxKind::Call),
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas,
            value,
            ..Default::default()
        }
    }
}

impl<'a> Arbitrary<'a> for TestTransactionRequest {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        // Rollup thresholds:
        //   base_fee        = 10 wei
        //   sender_balance  = 100_000_000_000_000_000 (1e17)
        //   tx_gas_limit    = 30_000_000
        //   intrinsic_gas   = 21_000 (simple ETH transfer)

        let max_fee_per_gas = u.choose(&[
            None,
            Some(0),
            Some(9),
            Some(10),
            Some(11),
            Some(10_000_000_000_000),
        ])?;

        let max_priority_fee_per_gas = if max_fee_per_gas.is_none() {
            u.choose(&[None, Some(0)])?
        } else {
            u.choose(&[None, Some(0), Some(11)])?
        };

        let gas = u.choose(&[None, Some(0), Some(20_999), Some(21_000), Some(30_000_001)])?;

        let values = [
            None,
            Some(U256::ZERO),
            Some(U256::from(1u64)),
            Some(U256::from(100_000_000_000_000_001u128)),
        ];
        let value = u.choose(&values)?;

        let to_options = [None, Some(Address::repeat_byte(0x22))];
        let to = u.choose(&to_options)?;

        Ok(TestTransactionRequest {
            max_fee_per_gas: *max_fee_per_gas,
            max_priority_fee_per_gas: *max_priority_fee_per_gas,
            gas: *gas,
            value: *value,
            to: *to,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum NonceOption {
    Below,
    Match,
    Future,
}

impl<'a> Arbitrary<'a> for NonceOption {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        u.choose(&[NonceOption::Below, NonceOption::Match, NonceOption::Future])
            .copied()
    }
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

impl<'a> Arbitrary<'a> for RegularTestAccount {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        u.choose(&[
            RegularTestAccount::Funded,
            RegularTestAccount::UnfundedUncovered,
        ])
        .copied()
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

impl<'a> Arbitrary<'a> for PaymasterTestAccount {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        u.choose(&[
            PaymasterTestAccount::FundedCovered,
            PaymasterTestAccount::UnfundedCovered,
            PaymasterTestAccount::UnfundedUncovered,
        ])
        .copied()
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

    // Primary consistency check: estimateGas and sendRawTransaction must agree.
    // Known expected divergences:
    //   - eth_call and createAccessList don't check affordability by design
    //   - when user provides explicit gas below the estimated gas, send fails (user error)
    //   - simulation doesn't validate nonce; Below/Future nonce may cause send to fail
    match (estimate_gas, call, create_access_list, send_raw_tx) {
        // All 4 agree OK
        (Ok(_), Ok(_), Ok(_), Ok(_)) => {
            println!("Consistent Ok");
        }
        // estimate and send agree OK; createAccessList may diverge
        (Ok(_), Ok(_), _, Ok(_)) => {
            if strict_check {
                assert!(
                    create_access_list.is_ok(),
                    "createAccessList should also succeed: {create_access_list:?}"
                );
            }
        }
        // All 4 agree Err
        (Err(_), Err(_), Err(_), Err(_)) => {
            println!("Consistent Err");
        }
        // estimate and send agree Err; call/createAccessList may succeed
        // (eth_call and createAccessList don't enforce affordability checks)
        (Err(_), _, _, Err(_)) => {
            println!(
                "estimate+send Err, call/createAccessList may diverge (no affordability check)"
            );
        }
        // estimateGas=Ok but sendRawTransaction=Err: allowed when user provided
        // explicit gas below the estimated amount (user sent with insufficient gas)
        (Ok(estimated), _, _, Err(_))
            if explicit_gas.is_some_and(|g| g < estimated.to::<u64>()) =>
        {
            println!(
                "estimate=Ok({estimated}) but send=Err: explicit gas {} < estimated (expected)",
                explicit_gas.unwrap()
            );
        }
        // estimateGas=Ok but sendRawTransaction=Err with non-Match nonce:
        // simulation doesn't validate nonce, so Below/Future nonce can cause send to fail
        (Ok(_), _, _, Err(_)) if !matches!(nonce_option, NonceOption::Match) => {
            println!(
                "estimate=Ok but send=Err with nonce={nonce_option:?} (simulation skips nonce check)"
            );
        }
        // estimateGas=Ok but sendRawTransaction=Err when fee fields were omitted:
        // estimateGas skips affordability preflight when no fee fields are present,
        // but the raw tx gets the base fee filled in and the sequencer checks affordability
        (Ok(_), _, _, Err(send_err))
            if explicit_max_fee.is_none()
                && send_err.to_string().contains(INSUFFICIENT_FUNDS_ERROR) =>
        {
            println!(
                "estimate=Ok but send=Err with max_fee=None (affordability preflight skipped without fee fields)"
            );
        }
        // True disagreement between estimateGas and sendRawTransaction
        _ => {
            panic!(
                "Responses disagree: \n\
                 {context}\n\
                 estimateGas={estimate_gas:?}\n\
                 call={call:?}\n\
                 createAccessList={create_access_list:?}\n\
                 sendRawTransaction={send_raw_tx:?}"
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Inner test functions
// ---------------------------------------------------------------------------

async fn test_regular_rollup_simulation_and_send_consistency(
    request: TestTransactionRequest,
    account: RegularTestAccount,
    nonce_option: NonceOption,
    strict_check: bool,
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let priv_key = account.priv_key();
    let ws_client = create_simple_storage_client(rollup.http_addr, priv_key).await;
    let signer: PrivateKeySigner = priv_key.parse()?;

    let mut request = request.build_tx_request(signer.address());
    apply_nonce(
        nonce_option,
        account.is_funded(),
        &ws_client,
        &signer,
        &mut request,
    )
    .await?;

    let context = format!(
        "account={account:?} nonce={nonce_option:?} to={:?} gas={:?} max_fee={:?} max_priority_fee={:?} value={:?} nonce_field={:?}",
        request.to, request.gas, request.max_fee_per_gas, request.max_priority_fee_per_gas, request.value, request.nonce
    );
    let results = call_all_endpoints(&ws_client, &request, &signer).await;
    check_consistency(
        &results,
        request.gas,
        request.max_fee_per_gas,
        nonce_option,
        strict_check,
        &context,
    );

    Ok(())
}

async fn test_paymaster_rollup_simulation_and_send_consistency(
    request: TestTransactionRequest,
    account: PaymasterTestAccount,
    nonce_option: NonceOption,
    strict_check: bool,
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup_with_selective_paymaster(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let priv_key = account.priv_key();
    let ws_client = create_simple_storage_client(rollup.http_addr, priv_key).await;
    let signer: PrivateKeySigner = priv_key.parse()?;

    let mut request = request.build_tx_request(signer.address());
    apply_nonce(
        nonce_option,
        account.is_funded(),
        &ws_client,
        &signer,
        &mut request,
    )
    .await?;

    let context = format!(
        "account={account:?} nonce={nonce_option:?} to={:?} gas={:?} max_fee={:?} max_priority_fee={:?} value={:?} nonce_field={:?}",
        request.to, request.gas, request.max_fee_per_gas, request.max_priority_fee_per_gas, request.value, request.nonce
    );
    let results = call_all_endpoints(&ws_client, &request, &signer).await;
    check_consistency(
        &results,
        request.gas,
        request.max_fee_per_gas,
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
    let test_tx_request = TestTransactionRequest {
        max_fee_per_gas: Some(10),
        max_priority_fee_per_gas: Some(0),
        gas: None,
        value: Some(U256::from(1u64)),
        to: Some(Address::repeat_byte(0x22)),
    };

    test_regular_rollup_simulation_and_send_consistency(
        test_tx_request,
        RegularTestAccount::Funded,
        NonceOption::Match,
        false,
    )
    .await
}

#[test]
#[should_panic(expected = "Responses disagree")]
fn omitted_max_fee_non_affordability_send_error_is_not_whitelisted() {
    let results = EndpointResults {
        estimate_gas: Ok(U64::from(21_000)),
        call: Ok("0x".to_string()),
        create_access_list: Ok(AccessListResult {
            access_list: Default::default(),
            gas_used: U256::from(21_000),
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
        None,
        None,
        NonceOption::Match,
        false,
        "synthetic omitted max_fee + non-affordability send error",
    );
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5))]

    #[test]
    fn proptest_simulation_send_consistency(bytes in prop::collection::vec(any::<u8>(), 64..256)) {
        let mut u = Unstructured::new(&bytes);
        let Ok(request) = TestTransactionRequest::arbitrary(&mut u) else {
            return Ok(());
        };
        let Ok(account) = RegularTestAccount::arbitrary(&mut u) else {
            return Ok(());
        };
        let Ok(nonce) = NonceOption::arbitrary(&mut u) else {
            return Ok(());
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                test_regular_rollup_simulation_and_send_consistency(
                    request,
                    account,
                    nonce,
                    false,
                )
                .await
                .unwrap();
            });
    }

    #[test]
    fn proptest_paymaster_simulation_send_consistency(bytes in prop::collection::vec(any::<u8>(), 64..1024)) {
        let mut u = Unstructured::new(&bytes);
        let Ok(request) = TestTransactionRequest::arbitrary(&mut u) else {
            return Ok(());
        };
        let Ok(account) = PaymasterTestAccount::arbitrary(&mut u) else {
            return Ok(());
        };
        let Ok(nonce) = NonceOption::arbitrary(&mut u) else {
            return Ok(());
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                test_paymaster_rollup_simulation_and_send_consistency(
                    request,
                    account,
                    nonce,
                    false,
                )
                .await
                .unwrap();
            });
    }
}
