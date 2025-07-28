use helpers::create_batch;
use rest_api_load_testing::Requests;
use sov_demo_rollup::MockDemoRollup;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::Spec;
use sov_modules_rollup_blueprint::RollupBlueprint;

pub type TestSpec = <MockDemoRollup<Native> as RollupBlueprint<Native>>::Spec;

const PRIVATE_KEYS_FILE: &str = "../../test-data/keys/token_deployer_private_key.json";
const URL: &str = "http://localhost:12346";

#[tokio::main]
async fn main() {
    let seq_da_address = "0000000000000000000000000000000000000000000000000000000000000000";
    let credentials_id = "0xfea6ac5b8751120fb62fff67b54d2eac66aef307c7dde1d394dea1e09e43dd44";

    let (address, token_id) = create_batch(PRIVATE_KEYS_FILE, URL.to_string()).await;

    let mut all_endpoints = Vec::new();
    all_endpoints.append(&mut ledger_endpoints());
    all_endpoints.append(&mut module_endpoints(
        &address,
        seq_da_address,
        credentials_id,
        &token_id,
    ));
    all_endpoints.append(&mut rollup_endpoints());

    let requests = Requests::new(URL, all_endpoints);
    let summary = rest_api_load_testing::start(requests).await;
    summary.print_summary();
}

fn ledger_endpoints() -> Vec<String> {
    // TODO: To test the commented endpoints, we need to send some transactions to the full node. #1807
    vec![
        "ledger/slots/latest",
        "ledger/batches/0",
        "ledger/batches/0/txs/0",
        "ledger/batches/0/txs/0/events/0",
        "ledger/events/0",
        //" ledger/slots/{slotId}/batches/{batchOffset}",
        // "ledger/slots/{slotId}/batches/{batchOffset}/txs/{txOffset}/events/{eventOffset}",
        "ledger/slots/0/events",
        "ledger/txs/0",
        "ledger/txs/0/events/0",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

fn module_endpoints(
    address: &<TestSpec as Spec>::Address,
    seq_da_address: &str,
    credentials_id: &str,
    token_id: &str,
) -> Vec<String> {
    // TODO: To test the commented endpoints, we need to send some transactions to the full node. #1807

    vec![
        // Accounts
        "modules/accounts/state/accounts".to_string(),
        format!("modules/accounts/state/accounts/items/{credentials_id}"),
        "modules/accounts/state/credential-ids".to_string(),
        format!("modules/accounts/state/credential-ids/items/{address}"),
        // Bank
        format!("modules/bank/tokens/?token_name={token_id}/balances/&sender={address}"),
        format!("modules/bank/tokens/{token_id}/total-supply").to_string(),
        // Nonces
        "modules/nonces/state/nonces".to_string(),
        format!("modules/nonces/state/nonces/items/{credentials_id}"),
        // Sequencer
        "modules/sequencer-registry/state/allowed-sequencers".to_string(),
        format!("modules/sequencer-registry/state/allowed-sequencers/items/{seq_da_address}"),
        "modules/sequencer-registry/state/preferred-sequencer".to_string(),
        // ChainState
        "modules/chain-state/state/genesis-da-height".to_string(),
        "modules/chain-state/state/inner-code-commitment".to_string(),
        "modules/chain-state/state/next-visible-rollup-height".to_string(),
        "modules/chain-state/state/next-visible-rollup-height".to_string(),
        "modules/chain-state/state/operating-mode".to_string(),
        "modules/chain-state/state/outer-code-commitment".to_string(),
        "modules/chain-state/state/slots".to_string(),
        "modules/chain-state/state/slots/items/0".to_string(),
        "modules/chain-state/state/state-roots".to_string(),
        "modules/chain-state/state/state-roots/items/0".to_string(),
        "modules/chain-state/state/time".to_string(),
        "modules/chain-state/state/true-rollup-height".to_string(),
        "modules/chain-state/state/true-to-visible-rollup-height-history".to_string(),
    ]
}

fn rollup_endpoints() -> Vec<String> {
    // TODO: To test the commented endpoints, we need to send some transactions to the full node. #1807
    vec![
        "rollup/base-fee-per-gas/latest".to_string(),
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1936
        // format!("rollup/addresses/{address}/dedup"),
        "rollup/sync-status".to_string(),
    ]
}

mod helpers {
    use std::path::Path;

    use anyhow::Context;
    use demo_stf::runtime::{Runtime, RuntimeCall};
    use sov_address::{EthereumAddress, FromVmAddress};
    use sov_bank::types::TokenIdResponse;
    use sov_cli::wallet_state::PrivateKeyAndAddress;
    use sov_modules_api::prelude::serde::de::DeserializeOwned;
    use sov_modules_api::transaction::Transaction;
    use sov_modules_api::{
        Address, Amount, CryptoSpec, PrivateKey, PublicKey, Runtime as RuntimeTrait, SafeVec, Spec,
    };
    use sov_test_utils::default_test_signed_transaction;

    use crate::TestSpec;
    const TOKEN_NAME: &str = "TestToken";

    struct Client(sov_api_spec::Client);

    impl Client {
        fn new(api_url: &str) -> Self {
            let client = sov_api_spec::Client::new(api_url);

            Self(client)
        }

        pub async fn send_transactions<S>(&self, transactions: &[Transaction<Runtime<S>, S>])
        where
            S: Spec,
            S::Address: FromVmAddress<EthereumAddress>,
        {
            let _ = self.0.send_txs_to_sequencer(transactions).await;
        }

        async fn get<T: DeserializeOwned>(&self, url: String) -> T {
            self.0
                .client()
                .get(url)
                .send()
                .await
                .unwrap()
                .json::<T>()
                .await
                .unwrap()
        }
    }

    fn build_create_token_tx(
        key: &<<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
        nonce: u64,
        initial_balance: Amount,
    ) -> Transaction<Runtime<TestSpec>, TestSpec> {
        let user_address: Address = key.pub_key().credential_id().into();

        let msg = RuntimeCall::Bank(sov_bank::CallMessage::<TestSpec>::CreateToken {
            token_name: TOKEN_NAME.try_into().unwrap(),
            token_decimals: None,
            initial_balance,
            mint_to_address: user_address.into(),
            admins: SafeVec::new(),
            supply_cap: None,
        });

        default_test_signed_transaction::<Runtime<TestSpec>, TestSpec>(
            key,
            &msg,
            nonce,
            &Runtime::<TestSpec>::CHAIN_HASH,
        )
    }

    pub(crate) async fn create_batch(
        private_key_file: &str,
        url: String,
    ) -> (<TestSpec as Spec>::Address, String) {
        let client = Client::new(&url);
        let keys =
            PrivateKeyAndAddress::<TestSpec>::from_json_file(Path::new(private_key_file), true)
                .context(format!("File does not exist: {private_key_file:?}"))
                .unwrap();
        let priv_key = keys.private_key;
        let sender = keys.address;
        let tx = build_create_token_tx(&priv_key, 0, Amount::new(100));
        client.send_transactions(&[tx]).await;
        let token_id_resp: TokenIdResponse = client.get(query_token_id(&url, sender)).await;

        (sender, token_id_resp.token_id.to_string())
    }

    fn query_token_id(url: &str, sender: <TestSpec as Spec>::Address) -> String {
        format!("{url}/modules/bank/tokens?token_name={TOKEN_NAME}&sender={sender}")
    }
}
