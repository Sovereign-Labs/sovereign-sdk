use alloy_primitives::Bytes;
use derive_more::Deref;
use ethereum_types::H160;
use ethers::core::abi::Address;
use ethers::core::types::transaction::eip2718::TypedTransaction;
use ethers::core::types::Eip1559TransactionRequest;
use ethers::providers::{Http, PendingTransaction};
use futures::StreamExt;
use sov_cli::NodeClient;
use sov_modules_api::{Runtime, Spec};
use sov_test_utils::SimpleStorageContract;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod rpc;
use rpc::RpcClient;

const GAS: u64 = 9000000u64;
const MAX_FEE_PER_GAS: u64 = 100;
const MAX_PRIORITY_FEE_PER_GAS: u64 = 1;

#[derive(Deref)]
pub struct TestClient {
    contract: SimpleStorageContract,
    node_client: NodeClient,
    pub nonce: Arc<AtomicU64>,
    #[deref]
    rpc_client: RpcClient,
}

impl TestClient {
    pub async fn new(
        private_key: &str,
        contract: SimpleStorageContract,
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
impl TestClient {
    fn make_tx(
        &self,
        to_address: Option<Address>,
        data: Option<ethers::core::types::Bytes>,
    ) -> TypedTransaction {
        let mut tx = Eip1559TransactionRequest::new()
            .from(self.address())
            .chain_id(self.rpc_client.chain_id())
            .max_priority_fee_per_gas(MAX_PRIORITY_FEE_PER_GAS)
            .max_fee_per_gas(MAX_FEE_PER_GAS)
            .gas(GAS);

        // Get next nonce atomically
        let nonce = self.nonce.load(Ordering::SeqCst);
        tx = tx.nonce(nonce);

        if let Some(data) = data {
            tx = tx.data(data)
        }

        if let Some(addr) = to_address {
            tx = tx.to(addr)
        }

        tx.into()
    }

    pub async fn send_tx(
        &self,
        tx: TypedTransaction,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        // Increment nonce
        let _ = self.nonce.fetch_add(1, Ordering::SeqCst);
        self.rpc_client.eth_send_transaction(tx).await
    }
}

impl TestClient {
    pub async fn deploy_contract(
        &self,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        let tx = self.make_tx(None, Some(self.contract.byte_code()));
        self.send_tx(tx).await
    }

    pub async fn deploy_contract_call(&self) -> Result<Bytes, Box<dyn std::error::Error>> {
        let tx = self.make_tx(None, Some(self.contract.byte_code()));
        self.eth_call(tx).await
    }

    pub async fn send_eth(&self, reciever: H160, eth_value: u128) -> PendingTransaction<'_, Http> {
        let mut typed_transaction = self.make_tx(Some(reciever), None);
        typed_transaction.set_value(eth_value);

        self.send_tx(typed_transaction).await.unwrap()
    }

    pub async fn set_value(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> PendingTransaction<'_, Http> {
        let tx = self.make_tx(
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );

        self.send_tx(tx).await.unwrap()
    }

    pub async fn set_values(
        &self,
        contract_address: H160,
        set_args: Vec<u32>,
    ) -> Vec<PendingTransaction<'_, Http>> {
        let mut requests: Vec<_> = Vec::with_capacity(set_args.len());

        for set_arg in set_args.into_iter() {
            let typed_transaction = self.make_tx(
                Some(contract_address),
                Some(self.contract.set_call_data(set_arg)),
            );

            requests.push(self.send_tx(typed_transaction).await.unwrap());
        }
        requests
    }

    pub async fn set_value_call_and_estimate_gas(
        &self,
        contract_address: H160,
        set_arg: u32,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let mut tx = self.make_tx(
            Some(contract_address),
            Some(self.contract.set_call_data(set_arg)),
        );
        let gas = self.rpc_client.eth_estimate_gas(tx.clone()).await;
        tx.set_gas(gas);

        self.rpc_client.eth_call(tx).await
    }

    pub async fn failing_call(
        &self,
        contract_address: H160,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let tx = self.make_tx(
            Some(contract_address),
            Some(self.contract.failing_function_call_data()),
        );
        self.rpc_client.eth_call(tx).await
    }

    pub async fn always_reverts(
        &self,
        contract_address: H160,
    ) -> Result<PendingTransaction<'_, Http>, Box<dyn std::error::Error>> {
        let tx = self.make_tx(Some(contract_address), Some(self.contract.always_revert()));
        self.send_tx(tx).await
    }

    pub async fn query_contract(
        &self,
        contract_address: H160,
    ) -> Result<ethereum_types::U256, Box<dyn std::error::Error>> {
        let typed_transaction =
            self.make_tx(Some(contract_address), Some(self.contract.get_call_data()));

        let response = self.rpc_client.eth_call(typed_transaction).await?;

        let resp_array: [u8; 32] = response.to_vec().try_into().unwrap();
        Ok(ethereum_types::U256::from(resp_array))
    }
}

// Rollup interactions
impl TestClient {
    pub async fn send_transactions_and_wait_slot<S: Spec, Rt: Runtime<S>>(
        &self,
        transactions: &[sov_modules_api::transaction::Transaction<Rt, S>],
    ) -> anyhow::Result<()> {
        let mut slot_subscription = self.node_client.client.subscribe_slots().await?;

        self.node_client
            .client
            .send_txs_to_sequencer(transactions)
            .await?;

        let _ = slot_subscription.next().await;

        Ok(())
    }
}

// Alloy
impl TestClient {
    pub async fn alloy_deploy_contract(&self) -> alloy_primitives::Address {
        let typed_transaction = self.make_tx(None, Some(self.contract.byte_code()));
        let addr = self
            .send_tx(typed_transaction)
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .contract_address
            .unwrap();

        alloy_primitives::Address::from_slice(addr.0.as_slice())
    }

    pub async fn alloy_set_value(
        &self,
        contract_address: alloy_primitives::Address,
        set_arg: u32,
    ) -> alloy_primitives::TxHash {
        let typed_transaction = self.make_tx(
            Some(ethers::core::abi::Address::from_slice(
                contract_address.as_slice(),
            )),
            Some(self.contract.set_call_data(set_arg)),
        );

        let tx_hash = self.send_tx(typed_transaction).await.unwrap().tx_hash();

        alloy_primitives::TxHash::from_slice(&tx_hash.0)
    }

    pub async fn alloy_emit_logs(
        &self,
        contract_address: alloy_primitives::Address,
        topic: u32,
        nb_of_logs: u32,
    ) -> alloy_primitives::TxHash {
        let typed_transaction = self.make_tx(
            Some(ethers::core::abi::Address::from_slice(
                contract_address.as_slice(),
            )),
            Some(self.contract.emit_logs(topic, nb_of_logs)),
        );

        let tx_hash = self.send_tx(typed_transaction).await.unwrap().tx_hash();

        alloy_primitives::TxHash::from_slice(&tx_hash.0)
    }
}
