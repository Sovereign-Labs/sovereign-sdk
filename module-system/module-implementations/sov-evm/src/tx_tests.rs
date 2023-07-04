use crate::evm::test_helpers::{make_contract_from_abi, test_data_path};
use ethers_core::{
    abi::Address,
    types::{transaction::eip2718::TypedTransaction, Bytes, Eip1559TransactionRequest},
    utils::rlp::Rlp,
    utils::Anvil,
};
use ethers_middleware::SignerMiddleware;
use ethers_providers::{Middleware, Provider};
use ethers_signers::{LocalWallet, Signer};
use std::str::FromStr;

#[tokio::test]
async fn tx_rlp_encoding_test() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
        .parse::<LocalWallet>()?;
    let from_addr = wallet.address();
    let to_addr = Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1")?;
    let data: Bytes = Bytes::from_str(
        "0x6ecd23060000000000000000000000000000000000000000000000000000000000000002",
    )?;

    let tx_request = Eip1559TransactionRequest::new()
        .from(from_addr)
        .chain_id(1u64)
        .nonce(0u64)
        .max_priority_fee_per_gas(413047990155u64)
        .max_fee_per_gas(768658734568u64)
        .gas(184156u64)
        .to(to_addr)
        .value(200000000000000000u64)
        .data(data);

    let tx = TypedTransaction::Eip1559(tx_request);

    let sig = wallet.sign_transaction(&tx).await?;
    sig.verify(tx.sighash(), wallet.address())?;

    let rlp_bytes = tx.rlp_signed(&sig);
    let rlp_encoded = Rlp::new(&rlp_bytes);

    let (decoded_tx, decoded_sig) = TypedTransaction::decode_signed(&rlp_encoded)?;
    decoded_sig.verify(decoded_tx.sighash(), wallet.address())?;

    assert_eq!(tx, decoded_tx);
    Ok(())
}

fn delpoy_data() -> Bytes {
    let mut path = test_data_path();
    path.push("SimpleStorage.bin");

    let contract_data = std::fs::read_to_string(path).unwrap();
    let contract_data = hex::decode(contract_data).unwrap();

    Bytes::from(contract_data)
}

fn update_contract(set_arg: ethereum_types::U256) -> Bytes {
    let mut path = test_data_path();
    path.push("SimpleStorage.abi");

    let contract = make_contract_from_abi(path);

    contract.encode("set", set_arg).unwrap()
}

fn get_data() -> Bytes {
    let mut path = test_data_path();
    path.push("SimpleStorage.abi");

    let contract = make_contract_from_abi(path);
    contract.encode("get", ()).unwrap()
}

#[tokio::test]
async fn send_tx_test() -> Result<(), Box<dyn std::error::Error>> {
    let chain_id: u64 = 1;
    let anvil = Anvil::new().chain_id(chain_id).spawn();

    let provider = Provider::try_from(anvil.endpoint())?;
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let client = SignerMiddleware::new_with_provider_chain(provider, key).await?;

    // Create contract
    let contract_address = {
        let from_addr = anvil.addresses()[0];

        let request = Eip1559TransactionRequest::new()
            .from(from_addr)
            .chain_id(chain_id)
            .nonce(0u64)
            .max_priority_fee_per_gas(413047990155u64)
            .max_fee_per_gas(768658734568u64)
            .gas(18415600u64)
            .data(delpoy_data());

        let typed_transaction = TypedTransaction::Eip1559(request);

        let receipt = client
            .send_transaction(typed_transaction, None)
            .await?
            .await?;

        receipt.unwrap().contract_address.unwrap()
    };

    // Call contract
    let set_arg = ethereum_types::U256::from(923);
    {
        let from = anvil.addresses()[0];
        let request = Eip1559TransactionRequest::new()
            .from(from)
            .to(contract_address)
            .chain_id(chain_id)
            .nonce(1u64)
            .max_priority_fee_per_gas(413047990155u64)
            .max_fee_per_gas(768658734568u64)
            .gas(18415600u64)
            .data(update_contract(set_arg));

        let typed_transaction = TypedTransaction::Eip1559(request);

        let _ = client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
            .await;
    }

    // Query contract
    {
        let from = anvil.addresses()[0];

        let request = Eip1559TransactionRequest::new()
            .from(from)
            .to(contract_address)
            .chain_id(chain_id)
            .data(get_data());

        let tx = TypedTransaction::Eip1559(request);

        let response = client.call(&tx, None).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        let get_arg = ethereum_types::U256::from(resp_array);

        assert_eq!(set_arg, get_arg)
    }

    Ok(())
}
