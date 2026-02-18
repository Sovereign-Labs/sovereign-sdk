# AGENTS.md - sov-ethereum

> See also: root `<repo>/AGENTS.md` for build commands, test runner, and general conventions.

## Crate Purpose

Thin JSON-RPC adapter that wraps `sov-evm` for the full node. Handles transaction submission, log queries with pagination, WebSocket subscriptions, and method stubs. This crate does NOT own EVM state — it delegates all state queries and gas estimation to `sov-evm::Evm`.

## Architecture Overview

| File | Description |
|------|-------------|
| `src/lib.rs` | RPC module registration, unsupported method stubs (~74 methods), error code constants, `EthRpcConfig` |
| `src/handlers/mod.rs` | Transaction submission (`eth_sendRawTransaction`, sync/realtime variants), authentication, metrics |
| `src/handlers/get_logs.rs` | `eth_getLogs` entry point, response size limiting |
| `src/handlers/get_logs/service.rs` | Log filtering logic, bloom pre-filter, cursor-based pagination |
| `src/handlers/get_logs/cursor.rs` | 20-byte cursor encoding (block_height + tx_index + log_index) |
| `src/handlers/subscribe.rs` | `eth_subscribe`/`eth_unsubscribe` entry |
| `src/handlers/subscribe/service.rs` | Streaming logic for newHeads (synthetic + real blocks) and logs |
| `src/handlers/subscribe/params.rs` | Subscription parameter validation |
| `src/handlers/subscribe/watermark.rs` | High-water mark tracking for deduplication |
| `src/signer.rs` | Local transaction signing (behind `local` feature) |

## Relationship to sov-evm

This crate delegates ALL state queries and gas estimation to `sov-evm::Evm`. It adds:

- **Transaction lifecycle**: RLP parse -> authenticate -> sequencer submit -> receipt callback
- **Log pagination**: cursor-based, with configurable size limits
- **WebSocket subscriptions**: synthetic block rate throttling at 200ms
- **Graceful shutdown**: via tokio watch channel

When implementing a new feature, ask: does this query EVM state? If yes, implement it in `sov-evm/src/rpc/handlers.rs`. This crate only handles sequencer interaction (tx submission, subscriptions) and log pagination.

## Error Codes (standardized in PR #2378)

| Code | Constant | Usage |
|------|----------|-------|
| `-32004` | `METHOD_NOT_SUPPORTED` | Stubbed/unsupported RPCs |
| `-32005` | `LIMIT_EXCEEDED` | Response too large (eth_getLogs) |
| `-32003` | `TX_REJECTED` | Transaction validation failed |
| `-32001` | `RESOURCE_NOT_FOUND` | Unknown block/receipt |
| `4` | `TIMEOUT` | eth_sendRawTransactionSync timeout |
| `-32602` | `INVALID_PARAMS` | Input validation failures |

Never invent new error codes. Use these constants or the `EthApiError` variants from `sov-rpc-eth-types`.

## Subscription Details

### newHeads
- Emits for both synthetic blocks (instant, throttled to 1 per 200ms) and real DA blocks.

### logs
- Filters via `alloy_rpc_types::Filter::matches()`, uses watermark deduplication.
- Only accepts `pending..pending` block range or default (no arbitrary ranges).

### Both subscriptions
- Watch `shutdown_receiver` for graceful termination.

## Testing

```bash
# Unit tests
cargo nextest run -p sov-ethereum <test_name>

# E2E tests (requires full rollup)
cargo nextest run -p sov-demo-rollup <test_name>

# List all tests
cargo nextest run -p sov-ethereum --list
```

| Location | Purpose |
|----------|---------|
| `src/handlers/get_logs/cursor.rs` | Cursor encoding unit tests |
| `src/handlers/subscribe/watermark.rs` | Watermark tracking unit tests |
| `src/handlers/subscribe/service.rs` | Subscription service unit tests |
| `<repo>/examples/demo-rollup/tests/evm/evm_logs.rs` | E2E log query tests |
| `<repo>/examples/demo-rollup/tests/evm/evm_subscribe.rs` | E2E subscription tests |
| `<repo>/examples/demo-rollup/tests/evm/evm_ws_watch.rs` | E2E WebSocket tests |

## When Adding New RPC Methods

1. **State query methods** (e.g., `eth_getBalance`, `eth_getCode`):
   - Implement in `sov-evm/src/rpc/handlers.rs` with `#[rpc_method]`
   - The method is automatically available; no changes needed in this crate unless you need to wrap it

2. **Sequencer interaction methods** (tx submission, subscriptions):
   - Implement in this crate's `src/handlers/`

3. **Unsupported methods**:
   - Add to the stub list in `src/lib.rs` to return `-32004` (`METHOD_NOT_SUPPORTED`) instead of `-32601` (method not found)

4. **Error mapping**:
   - Use `EthApiError` variants from `sov-rpc-eth-types`
   - Use the error code constants defined in `src/lib.rs`
