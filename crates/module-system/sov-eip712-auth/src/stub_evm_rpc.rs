use sov_modules_api::macros::config_value;

/// Create minimal EVM RPC module that allows wallets such as Metamask to connect to the rollup and
/// validate the `CHAIN_ID`, as they may refuse to sign an EIP712 request otherwise.
/// This function should be integrated into the rollup Runtime's `endpoints()` implementation as
/// such:
/// ```ignore
/// #[cfg(feature = "native")]
/// fn endpoints(api_state: sov_modules_api::rest::ApiState<S>) -> sov_modules_api::NodeEndpoints {
///     // Existing code to set up `axum_router` and `background_handles`...
///
///     let mut jsonrpsee_module = stf_declaration_crate::get_rpc_methods::<S>(api_state);
///     let stub_rpc = sov_eip712::stub_evm_rpc();
///     jsonrpsee_module.merge(minimal_evm_rpc).expect(
///         "Failed to merge minimal EVM RPC",
///     );
///     sov_modules_api::NodeEndpoints {
///         axum_router,
///         jsonrpsee_module,
///         background_handles
///     }
/// }
/// ```
pub fn stub_evm_rpc() -> jsonrpsee::RpcModule<()> {
    let mut minimal_evm_rpc = jsonrpsee::RpcModule::new(());

    // These two methods are needed for metamask to validate the chain ID, otherwise it refuses
    // to sign the EIP712 message
    minimal_evm_rpc
        .register_method("eth_chainId", |_, _, _| {
            let chain_id = config_value!("CHAIN_ID");
            Ok::<_, jsonrpsee::types::ErrorObjectOwned>(format!("0x{chain_id:x}"))
        })
        .expect("Failed to register eth_chainId");
    minimal_evm_rpc
        .register_method("net_version", |_, _, _| {
            let chain_id = config_value!("CHAIN_ID");
            Ok::<_, jsonrpsee::types::ErrorObjectOwned>(chain_id.to_string())
        })
        .expect("Failed to register net_version");

    // And the next two methods are necessary for Metamask to consider the RPC endpoint "live"
    // - without them it reports a connection error, making the network non-functional. But the
    // data does not matter for signing offchain messages, so dummy data is sufficient to keep
    // Metamask happy.
    minimal_evm_rpc
        .register_method("eth_blockNumber", |_, _, _| {
            // Return a placeholder block number since we don't have full EVM state
            // MetaMask just needs this to return successfully
            Ok::<_, jsonrpsee::types::ErrorObjectOwned>("0x1".to_string())
        })
        .expect("Failed to register eth_blockNumber");
    minimal_evm_rpc
        .register_method("eth_getBlockByNumber", |_, _, _| {
            // Return a minimal block structure
            let block = serde_json::json!({
                "number": "0x1",
                "hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                "timestamp": "0x0",
                "gasLimit": "0x1c9c380",
                "gasUsed": "0x0",
                "transactions": []
            });
            Ok::<_, jsonrpsee::types::ErrorObjectOwned>(block)
        })
        .expect("Failed to register eth_getBlockByNumber");

    minimal_evm_rpc
}
