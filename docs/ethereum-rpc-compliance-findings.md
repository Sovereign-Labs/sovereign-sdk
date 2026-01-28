# Ethereum JSON-RPC Compliance Findings (sov-evm + sov-ethereum)

Scope
- Code review: sov-evm RPC handlers and error mapping, sov-ethereum RPC wrapper handlers.
- Live probe: local RPC at http://localhost:12346/rpc (2025-02-14).

Summary
- Several common Ethereum RPC methods are missing.
- Multiple paths return non-standard error codes (notably -32001 or 500) for common client errors.
- Several parameter handling gaps diverge from standard JSON-RPC / execution-apis behavior.

Live probe results (key examples)
- web3_clientVersion -> -32601 Method not found (missing method).
- eth_getBalance [] -> -32602 Invalid params (missing params handled by jsonrpsee).
- eth_getBlockByNumber ["safe", false] -> -32602 invalid block number (string tag not supported).
- eth_getBlockByNumber [{"blockNumber":"0x1"}, false] -> -32602 invalid type (EIP-1898 object not supported).
- eth_sendRawTransaction ["0x"] -> -32001 Empty raw transaction (non-standard for invalid input).
- eth_sendRawTransaction ["0x01"] -> -32001 Deserialization failed (non-standard for invalid input).
- eth_feeHistory [0, "latest", []] -> code 500 (non-JSON-RPC), should be invalid params.
- eth_sendTransaction [{}] -> -32001 ETH_RPC_ERROR "No from address" (non-standard).

Missing methods (not implemented)
- web3: web3_clientVersion, web3_sha3.
- net: net_listening, net_peerCount.
- eth status/fees: eth_protocolVersion, eth_syncing, eth_coinbase, eth_mining, eth_hashrate, eth_maxPriorityFeePerGas.
- eth block/tx lookup: eth_getBlockTransactionCountByHash, eth_getBlockTransactionCountByNumber,
  eth_getTransactionByBlockHashAndIndex, eth_getTransactionByBlockNumberAndIndex,
  eth_getUncleCountByBlockHash, eth_getUncleCountByBlockNumber,
  eth_getUncleByBlockHashAndIndex, eth_getUncleByBlockNumberAndIndex.
- eth filters: eth_newFilter, eth_newBlockFilter, eth_newPendingTransactionFilter,
  eth_uninstallFilter, eth_getFilterChanges, eth_getFilterLogs.
- eth account/signing: eth_sign, eth_signTransaction, eth_signTypedData.
- misc: eth_getProof, eth_createAccessList.
- trace/debug/txpool: no trace_* APIs, no txpool_* APIs; debug only has debug_traceBlockByNumber/debug_traceTransaction.

Non-standard error codes (common errors)
- eth_sendRawTransaction invalid raw data -> -32001 (UNKNOWN_ERROR_CODE) instead of invalid input/params.
  Paths: make_raw_tx() + convert_to_tx_signed() -> ErrorObjectOwned::owned(-32001, ...).
  Files: crates/full-node/sov-ethereum/src/lib.rs, crates/module-system/module-implementations/sov-evm/src/evm/conversions.rs
- eth_sendTransaction missing from -> -32001 ETH_RPC_ERROR.
  File: crates/full-node/sov-ethereum/src/handlers/mod.rs
- eth_feeHistory block_count == 0 -> code 500 (not JSON-RPC).
  File: crates/module-system/module-implementations/sov-evm/src/rpc/fee_history.rs and rpc/error.rs
- Generic helper to_jsonrpsee_error_object always uses -32001 for many invalid-param cases
  (auth failures, log cursor errors, subscribe parameter errors, response size limit).
  Files: crates/full-node/sov-ethereum/src/lib.rs, handlers/mod.rs, handlers/get_logs.rs,
  handlers/get_logs/service.rs, handlers/subscribe/params.rs

Parameter handling gaps
- String block tags:
  - "safe" / "finalized" rejected by string parsing.
  - "latest" treated as "pending".
  File: crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs
- EIP-1898 blockId objects not supported in string-based methods.
  Affects: eth_getBlockByNumber, eth_getBalance, eth_getCode, eth_getStorageAt,
  eth_getTransactionCount, eth_getBlockReceipts, eth_call, eth_estimateGas.
  Files: crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs
- eth_call ignores state_override and block_overrides; eth_estimateGas does not accept overrides.
  File: crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs
- eth_call / eth_estimateGas ignore fee fields and transaction type (always EIP-1559, zero fees).
  File: crates/module-system/module-implementations/sov-evm/src/helpers.rs
- Pending semantics:
  - eth_getTransactionByHash and eth_getTransactionCount do not include pending txs even when
    "pending" is requested.
  Files: crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs, state_access.rs
- eth_subscribe logs rejects valid block ranges; only allows default or pending->pending.
  File: crates/full-node/sov-ethereum/src/handlers/subscribe/params.rs
- eth_feeHistory does not validate reward_percentiles (range/order).
  File: crates/module-system/module-implementations/sov-evm/src/rpc/fee_history.rs
- eth_sendTransaction (local) overwrites user-supplied gas with estimate.
  File: crates/full-node/sov-ethereum/src/handlers/mod.rs

Notes on expected vs current error mapping
- EthApiError maps several transaction/input errors to invalid params (-32602); typical clients
  use -32000 (Invalid Input) or -32003 (Transaction rejected) depending on the case.
  File: crates/utils/sov-rpc-eth-types/src/eth_api_error.rs
- into_rpc_error hardcodes code 500 (non-JSON-RPC), which leaks into eth_feeHistory and
  other EthApiError::other() call sites.
  File: crates/module-system/module-implementations/sov-evm/src/rpc/error.rs

Appendix: RPC methods present (non-exhaustive)
- sov-evm: net_version, eth_chainId, eth_getBlockByHash, eth_getBlockByNumber, eth_getBalance,
  eth_getStorageAt, eth_getTransactionCount, eth_getCode, eth_feeHistory,
  eth_getTransactionByHash, eth_getBlockReceipts, eth_getTransactionReceipt, eth_call,
  eth_blockNumber, eth_estimateGas, debug_traceBlockByNumber, debug_traceTransaction.
- sov-ethereum wrapper: eth_gasPrice, eth_sendRawTransaction, eth_sendRawTransactionSync,
  realtime_sendRawTransaction, eth_getLogs, eth_getLogsWithCursor, eth_subscribe,
  (local) eth_accounts, eth_sendTransaction.

