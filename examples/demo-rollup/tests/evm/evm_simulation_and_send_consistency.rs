use crate::evm::evm_test_helper::{
    call_all_endpoints, create_simple_storage_client, setup_test_rollup, EndpointResults,
    EVM_EXTENSION, SENDER_PRIV_KEY,
};
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, U256};
use alloy_rpc_types_eth::TransactionRequest;
use arbitrary::Arbitrary;
use arbitrary::Unstructured;
use proptest::prelude::*;

pub struct TestTransactionRequest {
    pub max_fee_per_gas: Option<u128>,
    pub max_priority_fee_per_gas: Option<u128>,
    pub gas: Option<u64>,
    pub value: Option<U256>,
    // TODO: Add to with 2 options: send to some address and create
}

impl TestTransactionRequest {
    pub fn build_tx_request(self, sender: Address) -> TransactionRequest {
        let TestTransactionRequest {
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas,
            value,
        } = self;
        TransactionRequest {
            from: Some(sender),
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas,
            value,
            ..Default::default()
        }
    }
}

impl<'a> arbitrary::Arbitrary<'a> for TestTransactionRequest {
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

        let max_priority_fee_per_gas = u.choose(&[None, Some(0), Some(11)])?;

        let gas = u.choose(&[None, Some(0), Some(20_999), Some(21_000), Some(30_000_001)])?;

        let values = [
            None,
            Some(U256::ZERO),
            Some(U256::from(1u64)),
            Some(U256::from(100_000_000_000_000_001u128)),
        ];
        let value = u.choose(&values)?;

        Ok(TestTransactionRequest {
            max_fee_per_gas: *max_fee_per_gas,
            max_priority_fee_per_gas: *max_priority_fee_per_gas,
            gas: *gas,
            value: *value,
        })
    }
}

async fn test_regular_rollup_simulation_and_send_consistency(
    request: TestTransactionRequest,
    sender_priv_key: &str,
    strict_check: bool,
    // TODO: Nonce params: Below, Valid, Future,
) -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_rollup_height_advance_by(1).await;

    let ws_client = create_simple_storage_client(rollup.http_addr, sender_priv_key).await;
    let signer: PrivateKeySigner = sender_priv_key.parse()?;

    let request = request.build_tx_request(signer.address());

    let results = call_all_endpoints(&ws_client, &request, &signer).await;

    let EndpointResults {
        estimate_gas,
        call,
        create_access_list,
        send_raw_tx,
    } = results;

    match (&estimate_gas, &call, &create_access_list, &send_raw_tx) {
        (Ok(_e), Ok(_c), Ok(_cal), Ok(_send)) => {
            println!("Consistent Ok");
            // TODO: Add estimated gas check, and other important stuff
        }
        (Ok(_e), Ok(_c), call_result, Ok(_send)) => {
            if strict_check {
                assert!(call_result.is_ok());
            }
        }
        (Err(_e), Err(_c), Err(_cal), Err(_send)) => {
            println!("Consistent Err");
            // TODO: Add error check match
        }
        (Err(_e), Err(_c), cal_result, Err(_send)) => {
            if strict_check {
                assert!(cal_result.is_err())
            }
        }
        _ => {
            panic!(
                "Responses disagree: \n\
            estimateGas={estimate_gas:?}\n\
            call={call:?}\n\
            createAccessList={create_access_list:?}\n\
            sendRawTransaction={send_raw_tx:?}
            "
            )
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn check_inner() -> anyhow::Result<()> {
    let test_tx_request = TestTransactionRequest {
        max_fee_per_gas: Some(10),
        max_priority_fee_per_gas: Some(0),
        gas: Some(21_000),
        value: Some(U256::from(1u64)),
    };

    test_regular_rollup_simulation_and_send_consistency(test_tx_request, SENDER_PRIV_KEY, false)
        .await
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5))]

    #[test]
    fn proptest_simulation_send_consistency(bytes in prop::collection::vec(any::<u8>(), 64..256)) {
        let mut u = Unstructured::new(&bytes);
        let Ok(request) = TestTransactionRequest::arbitrary(&mut u) else {
            return Ok(());
        };
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                test_regular_rollup_simulation_and_send_consistency(
                    request,
                    SENDER_PRIV_KEY,
                    false,
                )
                .await
                .unwrap();
            });
    }
}
