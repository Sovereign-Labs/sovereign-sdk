use std::str::FromStr;

use ethers_core::utils::{self, keccak256, Anvil};
use ethers_core::{
    abi::Address,
    k256::ecdsa::SigningKey,
    types::{
        transaction::eip2718::TypedTransaction, Bytes, Eip1559TransactionRequest,
        TransactionRequest,
    },
    utils::rlp::Rlp,
};
use ethers_middleware::SignerMiddleware;
use ethers_providers::{maybe, Middleware, MiddlewareError, PendingTransaction, Provider};
use ethers_signers::{LocalWallet, Signer};

use crate::evm::test_helpers::{make_contract_from_abi, test_data_path};

#[tokio::test]
async fn tx_test() -> Result<(), Box<dyn std::error::Error>> {
    let wallet = "dcf2cbdd171a21c480aa7f53d77f31bb102282b3ff099c78e3118b37348c72f7"
        .parse::<LocalWallet>()?;

    let addr = wallet.address();

    let tx_req = Eip1559TransactionRequest::new()
        .from(addr)
        .chain_id(1u64)
        .nonce(0u64)
        .max_priority_fee_per_gas(413047990155u64)
        .max_fee_per_gas(768658734568u64)
        .gas(184156u64)
        .to(Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1").unwrap())
        .value(200000000000000000u64)
        .data(
            Bytes::from_str(
                "0x6ecd23060000000000000000000000000000000000000000000000000000000000000002",
            )
            .unwrap(),
        );

    // create a transaction
    let tx = TypedTransaction::Eip1559(tx_req);

    // sign it
    let signature = wallet.sign_transaction(&tx).await?;
    let rlp_encoded = tx.rlp_signed(&signature);
    let rlp = Rlp::new(&rlp_encoded);

    let (actual_tx, signature) = TypedTransaction::decode_signed(&rlp).unwrap();
    signature
        .verify(actual_tx.sighash(), wallet.address())
        .unwrap();

    signature.verify(tx.sighash(), wallet.address()).unwrap();

    assert_eq!(tx, actual_tx);
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
async fn send_raw_tx() {
    let chain_id: u64 = 1;

    let anvil = Anvil::new().chain_id(chain_id).spawn();

    let provider = Provider::try_from(anvil.endpoint()).unwrap();
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(chain_id);

    let client = SignerMiddleware::new_with_provider_chain(provider, key)
        .await
        .unwrap();

    let contract_address = {
        let from = anvil.addresses()[0];

        let request = Eip1559TransactionRequest::new()
            .from(from)
            .chain_id(chain_id)
            .nonce(0u64)
            .max_priority_fee_per_gas(413047990155u64)
            .max_fee_per_gas(768658734568u64)
            .gas(18415600u64)
            .data(delpoy_data());

        let typed_transaction = TypedTransaction::Eip1559(request);

        println!("==");
        let receipt = client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        println!("==");
        receipt.contract_address.unwrap()
    };
    println!("contract_address {:?}", contract_address);

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

        println!("==");
        let receipt = client
            .send_transaction(typed_transaction, None)
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap();

        println!("receipt {:?}", receipt);

        println!();
        let received = client
            .get_transaction(receipt.transaction_hash)
            .await
            .unwrap()
            .unwrap();

        println!("received {:?}", received);
    }

    println!();
    {
        let from = anvil.addresses()[0];

        let request = Eip1559TransactionRequest::new()
            .from(from)
            .to(contract_address)
            .chain_id(chain_id)
            // .nonce(1u64)
            // .max_priority_fee_per_gas(413047990155u64)
            // .max_fee_per_gas(768658734568u64)
            // .gas(18415600u64)
            .data(get_data());

        let typed_transaction = TypedTransaction::Eip1559(request);

        let res = client.call(&typed_transaction, None).await.unwrap();

        let a: [u8; 32] = res.to_vec().try_into().unwrap();
        let x = ethereum_types::U256::from(a);
        println!("Res {:?} {:?}", res, x);

        assert_eq!(set_arg, x)
    }
}

/*
#[tokio::test]
async fn handles_tx_from_field() {
    let anvil = Anvil::new().spawn();

    let provider = Provider::try_from(anvil.endpoint()).unwrap();
    let key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
        .parse::<LocalWallet>()
        .unwrap()
        .with_chain_id(1u32);

    let acc = anvil.addresses()[0];
    let rec = provider
        .send_transaction(
            TransactionRequest::pay(key.address(), utils::parse_ether(1u64).unwrap()).from(acc),
            None,
        )
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();

    println!("RC {:?}", rec);

    let get = provider
        .get_transaction(rec.transaction_hash)
        .await
        .unwrap()
        .unwrap();

    println!("");
    println!("TX {:?}", get);

    //provider.send_raw_transaction(tx)

    let client = SignerMiddleware::new_with_provider_chain(provider, key)
        .await
        .unwrap();

    let request = TransactionRequest::new();
    /*
    // signing a TransactionRequest with a from field of None should yield
    // a signed transaction from the signer address
    let request_from_none = request.clone();
    let hash = *client
        .send_transaction(request_from_none, None)
        .await
        .unwrap();
    let tx = client.get_transaction(hash).await.unwrap().unwrap();
    assert_eq!(tx.from, client.address());

    // signing a TransactionRequest with the signer as the from address
    // should yield a signed transaction from the signer
    let request_from_signer = request.clone().from(client.address());
    let hash = *client
        .send_transaction(request_from_signer, None)
        .await
        .unwrap();

    let tx = client.get_transaction(hash).await.unwrap().unwrap();
    assert_eq!(tx.from, client.address());

    let addr = Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1").unwrap();
    assert_eq!(tx.from, addr);

    // signing a TransactionRequest with a from address that is not the
    // signer should result in the default anvil account being used
    /*    let request_from_other = request.from(acc);
    let hash = *client
        .send_transaction(request_from_other, None)
        .await
        .unwrap();
    let tx = client.get_transaction(hash).await.unwrap().unwrap();
    assert_eq!(tx.from, acc);*/*/
}*/

//call_past_state

//ethers-rs/ethers-middleware/src

/*


   /// Signs and returns the RLP encoding of the signed transaction.
   /// If the transaction does not have a chain id set, it sets it to the signer's chain id.
   /// Returns an error if the transaction's existing chain id does not match the signer's chain
   /// id.
   async fn sign_transaction(
       &self,
       mut tx: TypedTransaction,
   ) -> Result<Bytes, SignerMiddlewareError<M, S>> {
       // compare chain_id and use signer's chain_id if the tranasaction's chain_id is None,
       // return an error if they are not consistent
       let chain_id = self.signer.chain_id();
       match tx.chain_id() {
           Some(id) if id.as_u64() != chain_id => {
               return Err(SignerMiddlewareError::DifferentChainID)
           }
           None => {
               tx.set_chain_id(chain_id);
           }
           _ => {}
       }

       let signature =
           self.signer.sign_transaction(&tx).await.map_err(SignerMiddlewareError::SignerError)?;

       // Return the raw rlp-encoded signed transaction
       Ok(tx.rlp_signed(&signature))
   }

*/

//Breadcrumbsethers-rs/ethers-core/src/types/transaction eip2718.rs

/*
#[test]
    fn test_signed_tx_decode() {
        let expected_tx = Eip1559TransactionRequest::new()
            .from(Address::from_str("0x1acadd971da208d25122b645b2ef879868a83e21").unwrap())
            .chain_id(1u64)
            .nonce(0u64)
            .max_priority_fee_per_gas(413047990155u64)
            .max_fee_per_gas(768658734568u64)
            .gas(184156u64)
            .to(Address::from_str("0x0aa7420c43b8c1a7b165d216948870c8ecfe1ee1").unwrap())
            .value(200000000000000000u64)
            .data(
                Bytes::from_str(
                    "0x6ecd23060000000000000000000000000000000000000000000000000000000000000002",
                )
                .unwrap(),
            );

        let expected_envelope = TypedTransaction::Eip1559(expected_tx);
        let typed_tx_hex = hex::decode("02f899018085602b94278b85b2f7a17de88302cf5c940aa7420c43b8c1a7b165d216948870c8ecfe1ee18802c68af0bb140000a46ecd23060000000000000000000000000000000000000000000000000000000000000002c080a0c5f35bf1cc6ab13053e33b1af7400c267be17218aeadcdb4ae3eefd4795967e8a04f6871044dd6368aea8deecd1c29f55b5531020f5506502e3f79ad457051bc4a").unwrap();

        let tx_rlp = rlp::Rlp::new(typed_tx_hex.as_slice());
        let (actual_tx, signature) = TypedTransaction::decode_signed(&tx_rlp).unwrap();
        assert_eq!(expected_envelope, actual_tx);
        assert_eq!(
            expected_envelope.hash(&signature),
            H256::from_str("0x206e4c71335333f8658e995cc0c4ee54395d239acb08587ab8e5409bfdd94a6f")
                .unwrap()
        );
    }

 */
