use alloy_primitives::{Address, Bytes, TxHash, U256};
use alloy_rpc_types::{TransactionReceipt, TransactionRequest};
use derive_more::Deref;
use futures::StreamExt;
use sov_cli::NodeClient;
use sov_evm_test_utils::LegacySimpleStorage;
use sov_modules_api::{Runtime, Spec};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

mod provider_ext;
mod rpc;

pub use provider_ext::LogsWithCursorProvider;
pub use rpc::RpcClient;

const RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(100);

const GAS: u64 = 100_000_000u64;
const MAX_FEE_PER_GAS: u128 = 100;
const MAX_PRIORITY_FEE_PER_GAS: u128 = 1;

#[derive(Deref)]
pub struct SimpleStorageClient {
    pub contract: LegacySimpleStorage,
    node_client: NodeClient,
    pub nonce: Arc<AtomicU64>,
    #[deref]
    pub rpc_client: RpcClient,
}

impl SimpleStorageClient {
    pub async fn new(
        private_key: &str,
        contract: LegacySimpleStorage,
        http_addr: std::net::SocketAddr,
    ) -> Self {
        let rpc_client = RpcClient::new(private_key, http_addr).await;
        let node_client = NodeClient::new_at_localhost(http_addr.port())
            .await
            .unwrap();

        // Fetch initial nonce from the network
        let from_addr = rpc_client.address();
        let initial_nonce = rpc_client.eth_get_transaction_count(from_addr).await;
        let nonce = Arc::new(AtomicU64::new(initial_nonce));

        Self {
            contract,
            rpc_client,
            node_client,
            nonce,
        }
    }
}

// Tx/nonce utils
impl SimpleStorageClient {
    pub fn make_tx(&self, to_address: Option<Address>, data: Option<Bytes>) -> TransactionRequest {
        let nonce = self.nonce.load(Ordering::SeqCst);

        let mut tx = TransactionRequest::default()
            .from(self.address())
            .nonce(nonce)
            .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas_limit(GAS);

        if let Some(data) = data {
            tx = tx.input(data.into());
        }

        if let Some(addr) = to_address {
            tx = tx.to(addr);
        }

        tx
    }

    pub async fn send_tx(
        &self,
        tx: TransactionRequest,
    ) -> Result<TxHash, Box<dyn std::error::Error>> {
        // Increment nonce
        let _ = self.nonce.fetch_add(1, Ordering::SeqCst);
        self.rpc_client.eth_send_transaction(tx).await
    }

    /// Wait for a transaction receipt to be available.
    pub async fn wait_for_receipt(&self, tx_hash: TxHash) -> TransactionReceipt {
        loop {
            if let Some(receipt) = self.rpc_client.receipt(tx_hash).await {
                return receipt;
            }
            tokio::time::sleep(RECEIPT_POLL_INTERVAL).await;
        }
    }

    /// Send a transaction and wait for its receipt.
    pub async fn send_tx_and_wait(
        &self,
        tx: TransactionRequest,
    ) -> Result<TransactionReceipt, Box<dyn std::error::Error>> {
        let tx_hash = self.send_tx(tx).await?;
        Ok(self.wait_for_receipt(tx_hash).await)
    }
}

impl SimpleStorageClient {
    pub async fn deploy_contract(&self) -> Result<TxHash, Box<dyn std::error::Error>> {
        let tx = self.make_tx(None, Some(self.contract.byte_code()));
        self.send_tx(tx).await
    }

    pub async fn deploy_contract_call(&self) -> Result<Bytes, Box<dyn std::error::Error>> {
        let tx = self.make_tx(None, Some(self.contract.byte_code()));
        self.eth_call(tx).await
    }

    pub async fn send_eth(&self, receiver: Address, eth_value: U256) -> TxHash {
        let mut tx = self.make_tx(Some(receiver), None);
        tx = tx.value(eth_value);

        self.send_tx(tx).await.unwrap()
    }

    pub async fn set_value(&self, contract_address: Address, set_arg: u32) -> TxHash {
        let tx = self.make_tx(Some(contract_address), Some(self.contract.set(set_arg)));

        self.send_tx(tx).await.unwrap()
    }

    pub async fn set_values(&self, contract_address: Address, set_args: Vec<u32>) -> Vec<TxHash> {
        let mut tx_hashes = Vec::with_capacity(set_args.len());

        for set_arg in set_args.into_iter() {
            let tx = self.make_tx(Some(contract_address), Some(self.contract.set(set_arg)));
            tx_hashes.push(self.send_tx(tx).await.unwrap());
        }
        tx_hashes
    }

    pub async fn set_value_call_and_estimate_gas(
        &self,
        contract_address: Address,
        set_arg: u32,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let mut tx = self.make_tx(Some(contract_address), Some(self.contract.set(set_arg)));
        let gas = self.rpc_client.eth_estimate_gas(tx.clone()).await;
        tx = tx.gas_limit(gas);

        self.rpc_client.eth_call(tx).await
    }

    pub async fn failing_call(
        &self,
        contract_address: Address,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let tx = self.make_tx(
            Some(contract_address),
            Some(self.contract.failing_function()),
        );
        self.rpc_client.eth_call(tx).await
    }

    pub async fn always_reverts(
        &self,
        contract_address: Address,
    ) -> Result<TxHash, Box<dyn std::error::Error>> {
        let tx = self.make_tx(Some(contract_address), Some(self.contract.always_revert()));
        self.send_tx(tx).await
    }

    pub async fn query_contract(
        &self,
        contract_address: Address,
    ) -> Result<U256, Box<dyn std::error::Error>> {
        let tx = self.make_tx(Some(contract_address), Some(self.contract.get()));

        let response = self.rpc_client.eth_call(tx).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        Ok(U256::from_be_bytes(resp_array))
    }
}

// Rollup interactions
impl SimpleStorageClient {
    pub async fn send_transaction_and_wait_slot<S: Spec, Rt: Runtime<S>>(
        &self,
        transaction: &sov_modules_api::transaction::Transaction<Rt, S>,
    ) -> anyhow::Result<()> {
        let mut slot_subscription = self.node_client.client.subscribe_slots().await?;

        self.node_client
            .client
            .send_tx_to_sequencer_with_retry(&transaction)
            .await?;

        let _ = slot_subscription.next().await;

        Ok(())
    }
}

// Alloy methods
impl SimpleStorageClient {
    pub async fn alloy_deploy_contract(&self) -> Address {
        let tx = self.make_tx(None, Some(self.contract.byte_code()));
        let receipt = self.send_tx_and_wait(tx).await.unwrap();
        receipt.contract_address.unwrap()
    }

    pub async fn alloy_set_value(&self, contract_address: Address, set_arg: u32) -> TxHash {
        let tx = self.make_tx(Some(contract_address), Some(self.contract.set(set_arg)));
        self.send_tx(tx).await.unwrap()
    }

    pub async fn alloy_emit_logs(
        &self,
        contract_address: Address,
        topic: u32,
        nb_of_logs: u32,
    ) -> TxHash {
        let tx = self.make_tx(
            Some(contract_address),
            Some(self.contract.emit_logs(topic, nb_of_logs)),
        );
        self.send_tx(tx).await.unwrap()
    }
}
