# 2025-08-01
- #1460 Adds `chain_hash` to the `/rollup/schema` endpoint response, this requires passing the chain hash during endpoint initialization.

# 2025-07-30
- #1444 Updates in `TestRollup`

# 2025-07-23
- #3283 **BREAKING CHANGE** Update the mock `code-commitment` to 8 bytes. This change requires modifying the `chain_state_xx.json` genesis files accordingly.
- #3293 Extends `get_runtime_schema` to work with types which haven't implemented `Runtime` trait yet.

# 2025-07-21
- #3263 **BREAKING CHANGE** Update error object used for errors returned by REST API endpoints.
    Previously `{errors: [{$ERR_OBJ}], meta: {}}`, now we just return a single `{$ERR_OBJ}` directly.
- #3279 **BREAKING CHANGE** `sequencer.da_address` is removed from rollup_config.toml. 
- #3271 adds `sov-zkvm-utils` crate for making ZKVM build.rs smaller

# 2025-07-19
- #3252 **BREAKING CHANGE** Removed "wrapper" object returned by all REST API endpoints, data is now returned directly.
    Previously `{data: $DATA, meta: {}}`, so data was accessed as `responseJson.data.field`, now this is just `{$DATA}`, access the data directly: `responseJson.field`.
    The structure for errors has also changed, it is now just a single top-level error object: `{status: $STATUS, title: $TITLE, details: []}`.
    The error object itself is unchanged.
- #3258 **BREAKING CHANGE** Added `UniversalWallet` and `schemars::JsonSchema` traits to `Module::CallMessage`, `schemars::JsonSchema` trait to `Module::Event`, and `Clone` trait to `Module`. Quick note: If you parameterize your `CallMessage` or `Event` over `S` (for example, to include an address of type `S::Address`), you must add the `#[schemars(bound = “S: Spec”, rename = "MyEnum")]` attribute on top your enum definition. This is a necessary hint for `schemars`, a library that generates a JSON schema for your module’s API. Also note that the rename is also often required.

# 2025-07-18
- #3246 **BREAKING CHANGE** A minimum bond is now required for sequencer registration.
- #3257 Adds a helper function for the Runtime schema build script.

# 2025-07-17 
- #3251 **BREAKING CHANGE** Update the `Module` trait so that `Module::call and Module::genesis` return `anyhow::Result<()>` instead of `Result<(), ModuleError>`.
- #3249 This PR renames the SchemaGenerator trait to UniversalWallet, so that the trait and the macro have the same name for better DevX.


# 2025-07-11
- #3208 Upgrades SP1 to `5.0.8` to have matching rust toolchain
- #3237 Moves rust version check from guest build to adapter build for SP1
- #3234 Replaces `secp256k1` with `k256` for EVM related authentication.

# 2025-07-14
- #3225 Bank: A transfer to self succeeds only if the transfer amount does not exceed the current balance.
- #3164 Adds support for an optional `stop_at_rollup_height` parameter in the rollup.
- #3211 Moves the `schema` and `dedup` endpoint traits/router to a new `endpoints` module for better organization

# 2025-07-10
- #3200 Upgrade `backon` crate.

# 2025-07-09
- #3193 **BREAKING CHANGE** this PR moves the sequencer tx status endpoint from `/sequencer/txs/{hash}` to `/sequencer/txs/{hash}/status`. A new endpoint is added at `/sequencer/txs/{hash}/` which returns the transaction's soft confirmation info. This PR also fixes an off-by-one error which caused the sequencer's unstable events endpoint to return one more event than was requested. It also makes the non-breaking change of allowing sequencer tx subscriptions from any past tx number `/sequencer/txs/ws?start_from=123`.


# 2025-07-04
- #3169 Exposes exponential backoff configuration in celestia adapter's configuration.
- #3162 Removes the Hasher generic from the PublicKey trait's credential_id() method. (As well as making sure EthereumPublicKey's get hashed specifically with the Keccak256 hash.)
- #3167 Changes the format of forced registration transactions on the DA layer, adding a single discriminant byte to specify that the standard authenticator should be used. This makes their format identical to standard transactions.

# 2025-06-29
- #3123 Replaces `sov-value-setter` with `sov-synthetic-load` in sov-demo-stf.
- #3126 **BREAKING CHANGE** for running rollups: changes the format of the preferred sequencer PostgreSQL database. Any running rollups configured to run postgres in the sequencer will need to have the sequencer halted and the database wiped before restart. If there are pending soft-confirmations, they will be lost when the database is wiped.
This PR adds initial support for state replication across multiple sequencers sharing the same PostgreSQL database, as the initial stage of implementing failover. For those wishing to test the replication functionality, there is a temporary config value to launch sequencers in replica mode; be aware that this config value will be removed without warning in a subsequent PR once failover is implemented.

# 2025-06-27
- #3120 Adds optional `stop_at_rollup_height` flag to the rollup. This is not a breaking change.

# 2025-06-26
- #3106 **BREAKING CHANGE** adds a *required* batch_execution_time_limit_millis configuration to the preferred sequencer. The sequencer 
will not allow batches to continue growing once they reach this limit, even if that means causing downtime.
- #3110 Swaps `StorageConfig` with `RollupDbConfig`. Change should be transparent for rollup struct.
- #3112 adds validity distribution parameter to sov-soak generator
- #3114 **BREAKING CHANGE** Updates rust to 1.85
- #3137 **BREAKING CHANGE** Updates SP1 to 5.0.6

# 2025-06-25
- #3095 SOAK: inject MessageValidity as an argument to `run_generator_task_for_xx`

# 2025-06-24 
- #3083 Changes celestia rollup configuration in /demo-rollup to Operator mode. 
# 2025-06-19
- #2999 Reworks the sequencer's logic for opening/closing batches. Now the sequencer attempts to keep a few finalized slots in reserve rather than making them visible as quickly as possible. The number of such slots is configured by `ideal_lag_behind_finalized_slot` in the preferred sequencer config.
- #3070 This PR introduces breaking changes to the testing framework.
Access to genesis fields now requires calling the appropriate accessor methods. For examle before:
 `genesis_config.additional_accounts[0].clone();` now:
`genesis_config.additional_accounts()[0].clone();`

# 2025-06-19
- #3063 Add e2e tests for `OperatingMode::Operator`

# 2025-06-18
- #3061 Fixes an issue where the sequencer DB could become unreadable due to triggering a pathological case in RocksDB range deletions.

# 2025-06-17
- #3055 Integrates `OperatorIncentives` in `demo-rollup`.
- #3054 Adds `OperatingMode::Operator` to the rollup. This is NOT a breaking change for the consumers of the sdk.
- #3039 Implement a recovery mechanism in case the preferred sequencer goes down for long enough to invalidate soft-confirmations (i.e. `DEFERRED_SLOTS_COUNT` has passed, allowing deferred blobs to be included).
    * **BREAKING CHANGE**: New config option `sequencer.preferred.recovery_strategy`, with valid values `None` and `TryToSave`.
    * If the preferred sequencer detects that the visible slot number has gone past `DEFERRED_SLOTS_COUNT`, `None` will simply shut down the rollup; operators may then launch it again after either modifying the config option or wiping the contents of the preferred sequencer's database. `TryToSave` will instead immediately publish any pending soft-confirmations and cease accepting new transactions; this will save any soft-confirmations that are still valid, but the sequencer will be penalized for any that revert.

# 2025-06-13
- #3035 Improves logging during resync testing

# 2025-06-11
- #3006 Adds graceful shutdown handling when the rollup block executor encounters a panic. Instead of crashing unexpectedly, the sequencer now properly shuts down and rejects new transactions when the executor task fails.

# 2025-06-10
- #3015 bumps the `blob_processing_timeout_secs` in demo-rollup to allow more time for proof generation in the readme tests.
- #2971 internal in sov-soak-tests

- #3014 Adds `/modules/bank/tokens/gas_token` and `/modules/bank/tokens/gas_token/balances/{address}` endpoints to enable fetching the gas token id and gas token balances using the REST API endpoints.
- #3009 Add new transaction types to the value setter module that will enable us to load test the framework more comprehensively.
- #3000 Bumps the number of in-memory state transition infos to accommodate for the recent slow-down in zk proving in CI.

# 2025-06-08
- #2982 Removes `signer_address` from `[da]` section for celestia demo rollup, as this information can be fetched from the node.
  Users still can set this address for the self-check of the configuration.

# 2025-06-04
- #2981 Disable state root background checks. This change **should not be** adopted by Sovereign SDK customers. 
- #2979 Internal CI improvements
- #2990 Internal change in demo-rollup tests, related to updates in logging.

# 2025-05-30
- #2953 Added additional testing for reliable rollup resyncing with fresh nodes. No functional or breaking changes.

# 2025-05-29
- #2955 Adds new variant do the demo-rollup: MockDa rollup based NOMT storage.
- #2945 **BREAKING CHANGE** Adds `blob_processing_timeout_secs` filed in the rollup config.
- #2946 **BREAKING CHANGE** Changes the order of the generics on `ConfigurableSpec` and adds associated type defaults for `Storage` and `CryptoSpec`. 

```rust
pub type MockRollupSpec<M> = ConfigurableSpec<
    MockDaSpec,
    Risc0,
    MockZkvm,
    // Cryptospec used to go here
    MultiAddressEvm,
    M,
    // CryptoSpec is automatically set from the first Zkvm (in this case, Risc0CryptoSpec). Add a type here to override
    // Storage is automatically set to `ProverStorage<DefaultStorageSpec<<CryptopSpec as CryptoSpec>::Hasher>>;` if `native` mode is enabled and ZkStorage otherwise. Add a type here to override
  >;
```
- #2943 Adds **Configurable Transaction Processing Delays in Sequencer:** 
  - The `Runtime` trait in `sov-modules-api` now includes a `get_transaction_delay_ms(&self, call: &Self::Decodable) -> u64` method. Rollup developers can override this method to inspect the transaction and return a desired delay in milliseconds. The default implementation returns 0 (no delay).
  - This feature enables rollups to implement fine-grained control over transaction processing order and timing, potentially mitigating front-running or allowing for strategic ordering based on transaction type without complex pre-execution logic.

# 2025-05-27
- #2913 **BREAKING CHANGE** Add `Storage` generic parameter to `ConfigurableSpec`:
  ```rust
  use sov_state::{ProverStorage, ZkStorage};
  // Previous spec definition: 
  pub type MockRollupSpec<M> = ConfigurableSpec<MockDaSpec, Risc0, MockZkvm, Risc0CryptoSpec, MultiAddressEvm, M>;
  
  // Adding NativeStorage type
  type NativeStorage = ProverStorage<DefaultStorageSpec<<Risc0CryptoSpec as CryptoSpec>::Hasher>>;
  // New `ConfigurableSpec Definition 
  pub type MockRollupSpec<M> = ConfigurableSpec<
    MockDaSpec,
    Risc0,
    MockZkvm,
    Risc0CryptoSpec,
    MultiAddressEvm,
    M,
    NativeStorage,
  >;
  // And for ZKVM Guest:
  type Storage = ZkStorage<DefaultStorageSpec<<Risc0CryptoSpec as CryptoSpec>::Hasher>>;
  pub type MockZkSpec = ConfigurableSpec<
    MockDaSpec,
    Risc0,
    MockZkvm,
    Risc0CryptoSpec,
    MultiAddressEvm,
    Zk,
    Storage,
  >;
  ```

# 2025-05-24
- #2919 increases the suggested value of `max_allowed_node_distance_behind` to 10.
- #2921 Removes the `Fee` trait and associated fee estimation logic from `DaService` and its implementations (Celestia, Mock DA).
  - `DaService::send_transaction()` and `DaService::send_proof()` methods no longer accept a `fee` parameter.
  - DA fee calculation is now fully delegated to the respective DA layer nodes.
  - **Action Required for Starter Rollups:** Consumers of `DaService` (e.g., starter rollups, custom adapters) must update calls to `send_transaction` and `send_proof` to remove the fee argument.
- #2919 increases the suggested value of `max_allowed_node_distance_behind` to 10.

# 2025-05-23
- #2886 **BREAKING CHANGE** Renames the outdated `rollup_height` field to `slot_number`. This change modifies the `BatchResponse` struct and  introduces a breaking change for SDK clients.

# 2025-05-16
- #2886 Removes metadata lock from sov-metrics

# 2025-05-12
- #2862 There was an issue where `darling`, a dependency we use, introduced a breaking change that Cargo was auto-upgrading to. This required users to add a workaround of explicitly pinning the version using `darling = "=0.20.10"`. This has now been fixed and the workaround is no longer necessary.
- #2659 **BREAKING CHANGE** Removes `Signature` associated type from the `TransactionAuthenticator`
- #2863 increases `DEFERRED_SLOTS_COUNT` to 15000.

# 2025-05-05
- #2659 **BREAKING CHANGE** Added support for versioned transactions. SDK consumers must now use the new constructor `Transaction::with_call_v0`.
- #2823 adds a task to monitor critial background tasks for errors/early exits and will trigger a rollup shutdown if any are detected.
- #2845 adds a Celestia checker to the demo-rollup
- #2849 fixes celestia checker ranges.
- #2828 adds `max_concurrent_blobs` field to the sequencer config. This field is used to limit the number of blobs that can be submitted to the DA layer at once.

# 2025-05-02
- #2824 More aggressively checks for nondeterminism in the sequencer

# 2025-05-01
- #2821 simplifies several methods on `sov_metrics::MetricsTracker` by merging them into `MetricsTracker::submit` (and `submit_with_time`).
# 2025-04-30
- #2813 simplifies integration of the Ethereum JSON-RPC APIs into rollup code. See the new code for `demo-rollup` for usage examples.

# 2025-04-28
- #2804 fixes several instances of a sequencer syncing bug that manifests itself as `SequenceNumberTooLow` showing up in the logs.

# 2025-04-29
- #2808 **BREAKING CHANGE** Adds `max_batch_size_bytes` (set to 1MB) in `celestia_rollup_config.toml` & `mock_rollup_config.toml`. The preferred sequencer will reject transactions that would cause the batch to exceed the size limit.

# 2025-04-11
- #2739 renames some items inside `FullNodeBlueprint`. `ProofSerializer` becomes `ProofSender`, `create_proof_serializer` becomes `create_proof_sender` and has a new signature. The inner function body likely doesn't need to be changed.

# 2025-04-10
- #2720 adds an optional `slot_number` query parameter to all state queries.

# 2025-04-09
- #2727 adds a method `TransactionAuthenticator::compute_tx_hash` to compute the hash of a transaction. Such logic is already present inside `TransactionAuthenticator::authenticate`, but the new method isolates hash computation from authentication logic.
- #2728 adds a new associated type `Auth: TransactionAuthenticator<S>` to the `Runtime` trait, and two related methods (`wrap_call` and `allow_unregistered_tx`). These changes allow you to use externally defined `TransactionAuthenticator` implementations, notably `sov_modules_api::runtime::capabilities::RollupAuthenticator` and `sov_evm::EvmAuthenticator`. You can take a look `demo-rollup` for an example of the necessary changes to your `Runtime` trait implementation.
- #2736  **BREAKING CHANGE** attempts to derive `schemars::JsonSchema` on all event types by default. To upgrade, be sure you derive `JsonSchema` on your module's event type.
- #2729 `FullNodeBlueprint::create_endpoints` now requires a `shutdown_receiver` parameter. This is to allow for graceful shutdowns in websocket endpoints.
  Fixes a issue with the node hanging indefinitely on shutdowns with `CTRL+C`, etc.
- #2741 Marks `start_stop_zk_non_instant_finality` as flaky

# 2025-04-08
- #2718 raises the default value of `[sequencer.preferred] events_channel_size` to 10000. This makes the `/sequencer/events/ws` endpoint less prone to lagging and, as a consequence, unprompted disconnects.
# 2025-04-09
- #2727 adds a method `TransactionAuthenticator::compute_tx_hash` to compute the hash of a transaction. Such logic is already present inside `TransactionAuthenticator::authenticate`, but the new method isolates hash computation from authentication logic.

# 2025-04-07
- #2701 **BREAKING CHANGE**. `cors` field to the `runner.http_config` changed from `enabled = true`/`enabled = false` to `cors = "permissive"` or `cors = "restrictive"`.
  When `permissive` is a default.

# 2025-04-05
- #2699 incorporates the latest changes from the gas optimization workstream to the `sov-benchmark` crate.

# 2025-04-02
- #2687 Deprecates the sequencers `/batches` endpoint. There is no replacement for this endpoint because it's functionality (trigger a batch to be built & submitted to DA) is not intended to be user facing.
- #2661 **BREAKING CHANGE** changes the format of `Time` to a UNIX timestamp in milliseconds. In JSON config files, this now serializes as a number instead of an object. **You will need to update `chain_state.json` in your genesis config.**
- #2683 **BREAKING CHANGE** Removes `BlobType` from `HasKernel` trait.

# 2025-03-27
- #2649 renames several environment variables:
  1. `SOV_SDK_CONST_OVERRIDE_...` becomes `SOV_TEST_CONST_OVERRIDE_...`.
  2. `CONSTANTS_MANIFEST_TEST_MODE` becomes `SOV_TEST_MODE_CONST_MANIFEST`.
  3. `SOVEREIGN_SDK_EXPAND_PROC_MACROS` becomes `SOV_EXPAND_PROC_MACROS`.
  4. All benchmark-related env. vars. e.g. `BLOCKS`, `TXNS_PER_BLOCK`, `TIMER_OUTPUT` are now prefixed with `SOV_BENCH_`.
- #2648 fixes a bug that caused event keys to also contain the value part of the event for certain unit structs.

# 2025-03-25
- #2603 **BREAKING CHANGE**: Universal schema now supports borsh serialization. The following changes are breaking:
  * A new constant, `CHAIN_NAME`, was added. This gets saved in the schema and will be displayed to the user signing transactions, alongside the numeric `CHAIN_ID`.
  * The `Schema::of_rollup_types_with_chain_data()` constructor has been updated. The "metadata" generic has been removed and the argument type has been updated, as chains are expected to provide precisely a chain ID and a chain name, standardising this aspect.
  * Any tests utilizing the `Schema::of_single_type<T>(T)` constructor: this now returns a `Result<>` and will need to be unwrapped in the test.
Additionally, the chain has will change.
# 2025-03-26
- #2642 changes the behavior of `/sequencer/events/ws` so that events produced inside non-preferred batches are also visible.
# 2025-03-17
- #2615 Removes `max_allowed_blocks_behind` field from sequencer config, it was unused, simply remove this value from your configuration.

# 2025-03-13
- #2607 Removes `sov-test-modules`, which was accidentally exposed via `sov-test-utils`.
# 2025-03-17
- #2612 adds checks inside the sequencer to prevent the node falling too many blocks behind. The threshold is configured by a field inside the rollup config file.
    - The field is `[sequencer.max_allowed_node_distance_behind]`, a sensible default for this field is `10`.

# 2025-03-11
- #2602 Wallet schema: correctly handle explicit enum discriminants when the `#[borsh(use-discriminant=true)]` attribute is specified. (Previously this would result in incorrect serialization in the wallet.) This is not a breaking change but will alter the chain hash due to schema modifications to support this.
- #2600 removes database persistence for the `StandardSequencer`. Please use the `PreferredSequencer` instead if you need persistence, or reach out to the team if you have a need for it.

# 2025-03-10
- #2574 **BREAKING CHANGE**: Adds decimal point support to tokens in sov-bank.
    * The `CreateToken` callmessage now has a new `Option<u8>` parameter. If `None` is passed, the token will default to 8 decimal places.
    * All `TokenId`s will change, as they now encode the decimal places of the token. Any tests or scripts referencing existing `TokenId`s will need to be updated.
    * The chain hash and schema will change.
- #2578 Modified the wallet schema internal structure in preparation for improved `serde` support. This has no user-facing impact but will cause rollup chain hashes and generated schemas to change.
- #2521 **BREAKING CHANGE** for rollup buildscripts: added the `Address` type to the standard rollup universal schema. This allows wallets to present serialized addresses to the user in the rollup's preferred format. Build scripts will need to be adjusted to pass the 4th root type when creating the rollup's schema.
- #2582 Internal CI has been adjusted to ensure any wallet schema changes will be treated as breaking changes, starting in the near future. This is because schema changes a) reflect changes to the Transaction and RuntimeCall structs, potentially requiring adjustments to all users of a rollup; and b) changes to the schema hash invalidate all transactions signed using the old hash, requiring any transaction submitters to update to the new hash before continuing to use the rollup.
- #2584 modifies the return type of `sequencer_additional_apis` to return a `NodeEndpoints`. This makes it possible to serve all JSON-RPC requests on the same endpoint, and removes the need for `RpcModule`-to-`axum::Router` conversions.

# 2025-03-09
- #2583 **BREAKING CHANGE** Removes the `DaService::subscribe_finalized_header` method from the `DaService` trait. This method was only used in tests and not in the SDK.
- #2570 The `sov_universal_wallet::Schema::json_to_borsh` parsing functionality can now accept stringified numbers and booleans in the input JSON. For most types this is a convenience, but notably this allows 128-bit numbers to be passed around as strings in JSON and parsed correctly in wallets.
- #2567 removes the `flaky_` prefix from a handful of tests. No breaking changes.
- #2575 **BREAKING CHANGE FOR CELESTIA DA** - added support for authored blobs, new configuration parameter is needed:
  ```toml
  [da]
  celestia_rpc_auth_token = "MY.SECRET.TOKEN"
  celestia_rpc_address = "http://127.0.0.1:26658"
  max_celestia_response_body_size = 104_857_600
  celestia_rpc_timeout_seconds = 60
  # **New parameter**: Address of the sender. Should match celestia node configuration.
  signer_address = "celestia1a68m2l85zn5xh0l07clk4rfvnezhywc53g8x7s"
  ```

## 2025-03-07
- #2555 **BREAKING CHANGE** The method of calculating the credential ID for public keys that are 32 bytes in size has been updated. This is a breaking change, as all addresses used in tests must be updated. See the changes in the `.json` files in PR #2555.
- #2546 Adds a `#[sov_wallet(fixed_point(...))]` attribute to the UniversalWallet macro, enabling fixed point formatting for integers. Refer to the macro docs for more details. This is a non-breaking change.

## 2025-03-06
- #2546 Adds a `#[sov_wallet(fixed_point(...))]` attribute to the UniversalWallet macro, enabling fixed point formatting for integers. Refer to the macro docs for more details. This is a non-breaking change.
- #2545 **BREAKING CHANGE** requires any module methods that modify state to take `&mut self`, including `call` and `genesis`
- #2549 **BREAKING CHANGE** RPC handlers now listen on the same port as REST API, under path `/rpc`. Update of the configuraiton is needed:
  1. Section `[runner.rpc_config]` is removed.
  2. Section `[runner.axum_config]` is renamed to `[runner.http_config]`.

## 2025-03-04
- #2535 Reverts the changes from #2483 and enables the proof namespaces.
- #2536 Allows to run `sov-soak-testing` against demo-rollup on Celestia DA
## 2025-03-03
- #2588 makes the type signature of `FullNodeBlueprint::sequencer_additional_apis` stricter by adding some trait bounds.
- #2541 removes `blob_hash` and `da_transaction_id` fields from the response body of `POST /sequencer/batches`.
- #2522 includes git hash as a resource attribute in metrics so we know exactly what version of the rollup is running.
- #2533 Changes demo-stf dependency usage from path to workspace.
- #2537 Adjusts amounts in the testing framework to use `Amount` instead of u128
- #2539 Adds support for custom metrics tracking. Please refer to README in `sov-metrics` crate.

## 2025-02-26
- #2423 integrates the `AccessPattern` module into the demo-stf `Runtime`.

## 2025-02-25
- #2509 **BREAKING CHANGE** `max_fee` field in transaction details changed from `u128` to `Amount`, meaning number is encoded as string in JSON.
- #2513 **BREAKING CHANGE** `sequencer_bond` field in genesis of `sov-sequencer-registry` changed from `u128` to `Amount, meaning number is encoded as string in JSON.
- #2515 **BREAKING CHANGE** Events in `sov-seqeuencer-registry` and `sov-prover-incentives` now use Amount, meaning number is encoded as string in JSON.
- #2458 Configuration parameter for telegraf daemon now allows specifying explicitly if it is a UDP or TCP socket. Previously it was using only UDP. By default, it assumes UDP. 
  ```toml
  [monitoring]
  telegraf_address = "udp://127.0.0.1:8094"
  ```
## 2025-02-24
- #2498 **BREAKING CHANGE** Changes all token balances from `u64` to `u128`. This covers gas all references to balances or fund amounts throughout the SDK, including gas *prices* (though not gas units).
- #2497 Updates the PublicKey API and refines the conversion of public keys to Addresses. Introduces charging for CredentialId calculation.
- #2502 fixes the gas charging pattern computation. 

## 2025-02-22
- #2491 Burns the cost of deserializing blobs to reduce risk of spam by registered sequencers. 
- #2492 Allows setting the supply cap of a token during initialization. This cap cannot be modified later on, even by authorized minters. 

## 2025-02-20
- #2483 Disable the proof processing code path in the STF blueprint. See Issue #2487.
- #2457 adds a 2-phase withdrawal process to the sequencer registry. This requires splitting the `Exit` Callmessage into two calls. After this change, withdrawers must call `InitiateWithdrawal`, then wait for an unbonding period before calling `Withdraw`. This prevents transactions from breaking soft confirmations in some edge cases.

## 2025-02-21
- #2467 overhauls the internals of the `StorableMockDaLayer`, 
  reducing amount of disk access when querying head block or waiting for future block to be produced. No major user change.
  `runner.da_polling_interval_ms` can be safely reduced for mock_da rollups.
## 2025-02-19

- #2452 merges the `BatchBuilder` trait and the `Sequencer` struct into a single `Sequencer` trait. REST APIs are unchanged, but this is a breaking change for `sov_test_utils` such as `TestRollup` and `TestSequencerSetup`.
- #2449 overhauls the internals of the blob selector to hid sequencer balances from user space. This change is not primarily user facing, but the `authorize_sequencer` public method has been removed from `sov-sequencer-registry`

## 2025-02-18
- #2456 Fixes `sov-stf-runner` handling of DA layer re-org logic in case when the new chain is shorter than previous.
- #2448 Randomization now includes a new mandatory parameter for `MockDaConfig`: `reorg_interval = [n, m]`.
  This parameter defines how often reorganization-like randomization should occur.
  If you were previously using the `[da.randomization]` parameter, update your configuration to include `reorg_interval` to maintain the same behavior. Example:
  ```toml
  [da.randomization]
  seed = "0x0000000000000000000000000000000000000000000000000000000000000012"
  # **new section**: Reorg on every new block  
  reorg_interval = [1, 2]
  [da.randomization.behaviour.shuffle_non_finalized_blobs]
  drop_percent = 0
  ```
## 2025-02-17
- #2447 Bumps some gas constant values - in particular `MAX_SEQUENCER_EXEC_GAS_PER_TX` and `INITIAL_GAS_LIMIT`. Users of the SDK may need to update their genesis configuration file by increasing the sequencer bond inside the `sequencer_registry.json` config file.
- #2436 Adds the optional `DaService::get_block_header_at` method. 
  The default implementation relies on the less efficient `get_block_at` method. 
  Third-party DA adapters are encouraged to implement this method to improve performance.

## 2025-02-13
- #2426 **Breaking** Configuration for the sequencer now requires a rollup address to be specified.

  ```toml
  [sequencer]
  max_allowed_blocks_behind = 5
  da_address = "celestia1a68m2l85zn5xh0l07clk4rfvnezhywc53g8x7s"
  # This field was added! This is must be set to the address of the sequencer on the rollup.
  rollup_address = "sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7qhzze66"
  ```
```
## 2025-02-14
- #2428 The Paymaster module now supports a `transction_limit` configuration in policies. When set to `Some(n)`, the paymaster will only sponsor up to _n_ transactions from a given user. This is a break change for existing Paymaster module initialisation and test code which now needs the new field specified.

## 2025-02-07
- #2378 **Breaking**. Configuration for `MockDa` is changed. `block_producing` is now section. To keep periodic block producing, use the following configuration:
  ```toml
  [da]
  # No change
  connection_string = "sqlite://mock_da.sqlite?mode=rwc"
  # No change
  sender_address = "0000000000000000000000000000000000000000000000000000000000000000"
  # Removed:
  # block_producing = "periodic"
  # Moved to `da.block_producing.periodic` section:
  # block_time_ms = 1_000
  
  # New section:
  [da.block_producing.periodic]
  block_time_ms = 1_000
  ```
  Please refer documentation of `MockDaConfig` for more information about potential configuration options
## 2025-02-06
- #2368 Changes the computation of the hash passed to `begin_block_hook`. Now, that hash is from an older block, where the exact amount of the delay is configurable by a constant `STATE_ROOT_DELAY_BLOCKS`. If `STATE_ROOT_DELAY_BLOCKS` is zero, the behavior is identical to the old behavior. The default delay is currently set to 5 blocks. 
## 2025-02-07
- #2377 Removed the `credential_ids` mapping from the `sov-accounts` module. It was unused, but had been automatically exposed as aREST API, so any end-to-end API tests relying on it will need to be adjusted or removed.

## 2025-02-05
- #2367 gates `true_slot_number_at_height` behind the `"native"` feature and renames it to `true_slot_number_at_historical_height` to better reflect that only historical slot numbers (not the latest slots) are available.
- #2369 fixes `sov-benchmarks` following workspace unification + preferred sequencer fixes. This is not a breaking change.
- #2365 reorganizes `sov-benchmarks` following workspace unification
- #2363 removes `ExecutionContext`, `RollupHeight`, and `VisibleSlotNumber` from the transaction execution context `struct Context`. `RollupHeight` and `VisibleSlotNumber` can be accessed via `TxState` and other state accessors. The `ExecutionContext` information is not available anymore, please reach out to the team to submit a use case that requires it.

## 2025-02-04
- #2345 Refactor gas charging and for acessing values via StateWriter/StateReader: This is a breaking change due to updates in `constants.toml and constants.testing.toml`

## 2025-02-01
- #2337 fixes a bug that could cause transaction re-execution to fail in the preferred sequencer.
- #2359 cleans up `sov-benchmark`

## 2025-01-31
- #2333 Remove the notion of `ValidityCondition`s from the SDK, including DA blocks and the module system. This feature was (to our knowledge) unused - any code that used them may be safely deleted.

- #2331 Fixes memory consumption metrics. This may requires updating `risc0-zkvm` and `risc0-zk-platform` to version `1.2.1`
- #2335 Renames `SlotHooks` to `BlockHooks` to better reflect the timing of these hooks.

## 2025-01-27
- #2309 Improves metrics collection. This is not a breaking change for consumers of the SDK.

## 2025-01-22
- #2255 Makes it possible to generate benchmarks metrics using multiple processes.
## 2025-01-30
- #2320 makes several correctness improvements to the preferred sequencer's visible slot number increase logic. It also removes `SequencerTxSender` from `demo-rollup` and utilities.
## 2025-01-29
- #2315 Remove manual batch production from the `bank` integration tests.
- #2323 **Fixes the finalized slot WebSocket notification.** Now, it only sends the most recent slot information instead of resending every slot when a new one is finalized.
## 2025-01-28
- #2304 removes some mandatory arguments to `RollupBuilder::new` in favor of builder methods.
- #2310 allows to completely disable prover background test for in `RollupBuilder` for more precise testing.
- #2317 adds context to genesis initialization. **rollup-starter should have same update**, as it is an important usability fix.

## 2025-01-27
- #2309 Improves metrics collection. This is not a breaking change for consumers of the SDK.

## 2025-01-23
- #2270 adds logging to `sov-benchmarks`

- #2266 Add soak testing crate for long running tests
## 2025-01-22
- #2255 Makes it possible to generate benchmarks metrics using multiple processes.
- #2259 removes the generic `AuthorizationData` associated type from traits (e.g. `TransactionAuthenticator`, `HasCapabilities`) and replaces it with a single concrete `AuthorizationData<S>` throughout the codebase.
- #2267 adds `sov-cli node wait-for-aggregated-proof` command that allows to wait for next aggreggated proof to be retrievable from the node.

## 2025-01-21
- #2252 Adjust the `/dedup` API endpoint to return the next unused generation rather than the next available nonce. This follows from the earlier (#2182) that made generations the default deduplication/uniqueness mechanism for native sovereign transactions. EVM transactions still use nonces (for standard compatibility) and those can be queried using the ethereum RPC.
Note that the recommended way to deduplicate transaction now is using the current UNIX timestamp in seconds as the generation. The `/dedup` endpoint is intended for compatibility with rollup-agnostic clients (enabling the same transaction submission flow for both nonce-based and generation-based rollups), or for state introspection.
- #2247 exports `sov_modules_rollup_blueprint::logging::should_init_open_telemetry_exporter` so there's a standard way to ensure that Open Telemetry exporter should be enabled.
- #2245 Makes state that is *only* soft-confirmed unavailable via archival queries (i.e. API queries that include the `?rollup_height` parameter). State now becomes available via archival APIs at the same time across all node, regardless of whether the node is also providing soft confirmations. This fixes a bug where the archival APIs could return incorrect data when queried for soft-confirmed state.

- #2223 unifies crates within the `sovereign-sdk[-wip]` repository into a single workspace. This changes the way SDK maintainers' local workflow, but doesn't impact downstream SDK consumers. See `cargo switcheroo --help` for more information, and make sure you update your Rust Analyzer settings as shown in `.vscode/settings.default.json`.
## 2025-01-20
- #2246 Adds a parameter to the `PrivateKeyAndAddress::from_json_file` method which allows skipping the check that the deserialized address matches the key's default value.
- #2245 Makes state that is *only* soft-confirmed unavailable via archival queries (i.e. API queries that include the `?rollup_height` parameter). State now becomes available via archival APIs at the same time across all node, regardless of whether the node is also providing soft confirmations. This fixes a bug where the archival APIs could return incorrect data when queried for soft-confirmed state.
- #2231 revert formatting of `GasUnit` when serializing with universal wallet. Allows keeping the array/tuple format instead of object structure.
- #2230 The ValueSetter::SetValue call message was updated to include an optional gas parameter, which will be used for updating the value.

## 2025-01-16
- #2222 disables tests for prover database clogging
- #2226 merge `sov-cycle-utils` with `sovereign-sdk-wip`. This may be a breaking change if you were using the `cycle_tracker` proc-macro decorator. In that case, you may import the `cycle_tracker` macro from either `sov-modules-api` or `sov-modules-macros` instead. Besides, you don't need to specify `target_os = "zkvm"` anymore when using the `cycle_tracker` decorator.
## 2025-01-15
- #2220 integrates gas constant tracking inside `sov-benchmarks`. This is not a breaking change for consumers of the SDK.

## 2025-01-14
- #2214 adds a way to track gas constants usage in the SDK. Consumers of the SDK may use the `track_gas_constants_usage` proc-macro decorator to gather metrics relevant to gas constants usage within the SDK. This is not a breaking change.

## 2025-01-13
- #2207 makes it possible to track gas constant consumption within the SDK. Consumers of the SDK can now track gas consumption within the SDK by using the `gas_constant_estimation` feature flag and adding a name tag to the constants they want to track.
## 2025-01-13
- #2207 makes it possible to track gas constant consumption within the SDK. Consumers of the SDK can now track gas consumption within the SDK by using the `gas_constant_estimation` feature flag and adding a name tag to the constants they want to track.
## 2025-01-15
- #2218 enables `[sequencer] automatic_batch_production` by default. This is a breaking change if you're relying on manual batch production (`POST /sequencer/batches`), and we suggest migrating your tests and deployments to use automatic batch production. If necessary, the option can be disabled to revert to the previous behavior.
- #2216 drops support for `cargo test` when working on the SDK repository. Use `cargo nextest` instead, which runs one test per process. `cargo test` is still supported for downstream SDK consumers.

## 2025-01-09
- #2173 Separates the notion of a `rollup_height` from that of a `slot_number`. The current `slot_number` is simply the current DA block number minus the DA height at the rollup's genesis. The `rollup_height` is the number of logical rollup blocks that have been created - in other words, `rollup_height` matches the slotnumber for based rollups, while it increments by `1` for each batch sent by the preferred sequencer in soft-confirming rollups. Starting from this PR, rollup *state* is queried by the new `rollup_height` rather than `slot_number`. Ledger state is still queried by `slot_number`. A future PR will make ledger state queriable by rollup height as well.
- #2162 migrates the SDK to Rust 1.81. Compatibility with older `rustc` versions is not guaranteed.
- #2183 adds a way to collect and store metrics from influxDB. This is not a breaking change for customers of the SDK.
- #2182 Added generations as a replacement to nonces. EVM transactions are unaffected, but all other transactions must now use per-account generation numbers.
  - Multiple transactions can have the same generation, as long as they are different (i.e. have different hashes). Generations older than a configurable limit are pruned. Transactions with a generation number below the pruned limit are automatically considered invalid. Storing too many transaction hashes increases cost; regular pruning is encouraged by regularly increasing the generation number. Using the current UNIX timestamp in seconds is a convenient way to set the generation number in most cases.
  - All references to `nonce` are replaced with `generation` when constructing transactions and in API calls. As mentioned above, this excludes EVM transactions (which continue to use `nonce`s and retain full EVM compatibility).
  - Any integration tests that relied on nonces automatically incrementing, or duplicate or non-consecutive nonces being invalid, will exhibit changed behavior and will need to be adjusted.

## 2025-01-08
- #2170 increases the suggested value for `DEFERRED_SLOTS_COUNT` to 50. This value is more appropriate than the previous value (5) for testing, but still too low for production use. A value in the order of 1000 is more appropriate for production.

## 2025-01-07
- #2164 adds a way to assert the logs against the state in the benchmark generation.
- #2147 Add handling for ignored transactions in the STF blueprint. No breaking change for customers of the SDK.
- #2166 fixed batch submission to sequencer inside `sov-benchmarks`. This isn't a breaking change for customers of the SDK.
- #2030 is a major overhaul of internals. Notable changes are:
- #2030 is a major overhaul of sequencer internals. Notable changes are:
  - Removal of the `should_update_state` configuration option. If you previously enabled it, you can safely remove it.
  - Slight changes to error messages and error objects in the `/sequencer` REST API to provide more helpful information.
  - Fixes a major memory leak and performance problem inside the preferred sequencer that would cause it to slow down to a crawl.
  - Fixes several race conditions that would cause REST API state to be momentarily inconsistent or outdated briefly after accepting transactions.

## 2025-01-05
- #2142 adds execution capabilities to the benchmark generation. This is not a breaking change for customers of the SDK.
## 2025-01-06
- #2145 fixes various risc0 related issues in sov-rollup-interface, risc0 adapter and celestia adapter
## 2025-01-05
- #2141 adapts the `sov-value-setter` module to benchmark generation. This is not a breaking change for customers of the SDK.
- #2146 fixes CI and licenses for guest provers

## 2025-01-04
- #2137 introduces the FromVmAddress trait bounds to demo-rollup (which includes an EVM module), allowing EVM transactions to be submitted natively from EVM addresses—without requiring the registration of EVM addresses as credentials.

## 2025-01-03
- #2120 adds benchmark generation capabilities to `sov-benchmark` through a CLI. This is not a breaking change for customers of the SDK.

## 2025-01-01
- #2127 Removes nested `StateMap`s from existing SDK modules

- #2123 introduces newtypes `sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber}` which are used throughout the SDK codebase for better type safety and readability. Method type signatures for `VersionedStateValue` and `VersionedStateVec` use these newtypes which is a breaking change for kernel modules.

- #2130 Extends `SKIP_GUEST_BUILD` variable usage. Please checkout PR if you need to port this functionality to your setup.
- #2126 Makes tests more stable.
- #2125 Upgrades tokio to 1.42 to fix bug https://github.com/tokio-rs/tokio/issues/6839
- #2132 Fixes flakiness in sov-demo rollup
## 2024-12-30
- #2110 adds `Display` and `FromStr` bounds on the keys of `StateMap`s

## 2024-12-26
- **Breaking Change** #2095 The `Transaction` type no longer implements `borsh::BorshSerialize`. Transaction is now deserializable only via the `MeteredBorshDeserialize` trait.
- #2094 Move `DispatchCall::decode_call` method to `Runtime`. This is not a breaking change since the method was not used outside of the sequencer.
## 2024-12-25
- **Breaking Change** #2093 Changes the signature of 
`TransactionAuthenticator::authenticate_unregistered` method. Now the method takes `BatchFromUnregisteredSequencer` instead of `Self::Input` as an argument.
- **Breaking Change** #2092 Renames `TransactionAuthenticator::parse_input` to `TransactionAuthenticator::decode_serialized_tx`
- **Breaking Change** #2091 Changes the `TransactionAuthenticator` trait. Now the `parse_input & authenticate`  methods accept `FullyBakedTx`.
This is a breaking change for the consumers of the SDK.
- **Breaking Change** 2089 Changes standard address length to 28 bytes. All rollup addresses (previously 32 bytes long) are now standardized to 28 bytes. This includes typed addresses, Bech32 string representations, default addresses in test data, and configuration files referencing addresses. This was done so that our rollups can support VMs such as EVM, SVM, MoveVM natively alongside standard modules.
## 2024-12-23
- #2083 Adds a new `MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE` constant in the constants.toml file. This sets the maximum size of data that can be returned by the blob selector.
## 2024-12-20
- #2069 Enable support for sending proofs and batches in parallel. This is a breaking change, requiring SDK consumers to update their code to align with the new API. In most cases, this involves modifying the `da.send_transaction(..).await?` call to `da.send_transaction(..).await.await??`.
- #2067 Move the retry logic to CelestiaDaService, and remove the DaServiceWithRetries type.
## 2024-12-19
- #2075 Add UniversalWallet derive to the crypto types for Risc0 and SP1 zkvms. Previously the macro was only derived for types in the MockZkvm. This made it impossible to derive the universal schema on the `Transaction<S: Spec>` type with a Spec that used a real zkvm - now fixed.
- #2064 Renames `Runtime::GenesisPaths` to `Runtime::GenesisInput`. This field is generic and can be anything, not just a struct with a set of paths.
## 2024-12-16
 - #2049 removes `StateItem::new` method to increase safety of the `StateItem` API.
## 2024-12-15
- #2048 removes the `offchain` macro, as well as the outdated modules `sov-nft` (in module-system/module-implementations) and `simple-nft-module` (in the examples directory).
- #2021 removes prometheus tracking from STF runner. Initialization of Prometheus registry is kept for other crates.
## 2024-12-06
- #1997 Removes `AggregatedProofPublicData` from the public API, making it accessible only after proof verification.
## 2024-12-05
- #1978 changes the response format of `POST /sequencer/batches`. `da_height` and `num_txs` were removed, it now contains `tx_hashes`.
- #1996 Renames `authorized_minters` field to `admins` for bank related call messages and opearations. This field is also used to determine who can freeze tokens, `admin` better encompasses the users full privlidges.
- #1989 Added template transaction support to the universal schema. This is not a breaking change, but is now available for use - refer to the macro documentation for attribute syntax. Templates will be available for use to wallets based on the universal schema, as a standard method of executing predefined transaction types (such as token transfers) for dapps and frontends unaware of the internal structure of a specific rollup's call messages.
- #2003 Advisory: UniversalWallet is now derived by default when using the `DispatchCall` macro (i.e. on all normal `Runtime` definitions). While unlikely, it is possible that this could introduce unexpected breakage.

## 2024-12-04
- #1963 Upgrades `sov-celestia-adapter` to the latest client version. Please make sure running against >= `celestia-node:v0.20.4-mocha`
- #1964 Fix incorrect value in `sov-bank` event field `TokenCreated::minter`. This was incorrectly set to `mint_to_address`. Adds a `mint_to_address` field to the event to capture this value.

## 2024-11-29
- #1948 Makes `sov-metrics` crate native only. Explicit feature gating is required in places where zk guest mode is possible.
## 2024-11-27
- #1903 makes significant changes to `demo-rollup` integration tests, which now rely on the `sov_test_utils::test_rollup::RollupBuilder` utility. Migrating similar tests must be done through the new APIs, which are simpler and provide more flexibility and control.
## 2024-11-20
- #1889 makes `demo-rollup` use the `SoftConfirmationsKernel` by default. This may be a breaking change for the consumers of the SDK that rely on the default behavior of the `demo-rollup`. Please make sure to update the `blob` format to `PreferredBlobData` if sending batches using the preferred sequencer in the `demo-rollup`.
## 2024-11-26
-  #1931 Remove the `salt` query parameter from /modules/bank/tokens
## 2024-11-25
- #1925 makes several improvements to sequencer stability after restarts.
## 2024-11-26
- #1921 adds new `tracing` utilities in `sov_modules_rollup_blueprint::logging`. Most rollups will be able to seamlessly switch from their own custom implementation of `fn initialize_logging` to `sov_modules_rollup_blueprint::logging::initialize_logging`, which can be used to setup logging inside the rollup node's `main` function.
## 2024--11-21
- #1899 Adds `monitoring` section to rollup configuration. 
  Mandatory field is `telegraf_address` which is most likely to be "127.0.0.1:8094".
  Other fields are described in demo-rollup configuration files.
## 2024-11-22
- #1920 Remove metadata generic from universal schema, moving it to the schema constructor. Metadata is now pre-hashed when stored. Wallets and other schema users no longer need to provide the correct generic type when deserializing a schema. However, constructing the schema requires passing the metadata type.
## 2024-11-19
- #1797 changes the `Transaction` and `UnsignedTransaction` types to make them generic on their call message type. This has cascading effects on many internal structures.
  - Module developers will need minimal changes, except if construction `Transaction`s in integration tests. If your tests follow the same template as Sovereign's tests and use the same testing framework, refer to the new structure of our existing modules; for example, `create_plain_message<M: Module>(M::CallMessage)` now requires an additional Runtime generic, with its new signature being `create_plain_message<R: Runtime, M: Module>(M::CallMessage)`.
  - Any other code using `Transaction`/`UnsignedTransaction` types will need to specify the call type. In contexts where a runtime is available, the corresponding `Runtime` will be the correct type; in generic code, the `DispatchCall` trait is the correct bound in most cases.
  - Conversely, call messages never need to be serialized before inclusion in a transaction. A `M::CallMessage where M: Module` can be converted to a runtime's `RuntimeCall` type using `<R as EncodeCall<M>>::to_decodable(your_call_message)`.

## 2024-11-13
- #1873 changes the type of `Bank::CreateToken.authorized_minters` from `Vec` to `SafeVec`. This type can be easily constructed by calling `try_into()` on an existing `Vec` as long as that `Vec` is not larger than the new size bound (20 items).
- #1882 adds `max_number_of_transitions_in_db` and `max_number_of_transitions_in_memory` to proof_manager config.
## 2024-11-14

- #1880 removes `FullNodeBlueprint::get_operating_mode` and replaces it with a new method `Runtime::operating_mode`. `Runtime::GenesisConfig` types now need to implement `Clone`. Minor API changes to testing utilities used inside `demo-rollup/tests/`.

## 2024-11-13

- #1870 removes many of the testing utilities inside `demo-rollup/tests/test_helpers.rs`, and replaces them with a new utility `sov_test_utils::test_rollup::RollupBuilder`. You can take a look at `demo-rollup/tests/bank` for some usage examples.

## 2024-11-12
- #1861 Moves the `Runtime` trait to `sov-modules-api`. It remains re-exported from `sov-modules-stf-blueprint`, so most usages remain unaffected. However, the `RuntimeEndpoints` struct is also moved and is not re-exported, so any code importing `sov-modules-stf-blueprint::RuntimeEndpoints` must be changed to import `sov-modules-api::RuntimeEndpoints`.

## 2024-11-05
- #1821 removes the associated type `TxState` from `TxHooks` and adds two new generic parameters to `ApplyBatchHooks::begin_batch_hook` and `ApplyBatchHooks::end_batch_hook`. Please refer to `hooks_impl.rs` for a usage example using the new trait signatures.
- #1819 Derive `JsonSchema` on runtime calls. All types used in call messages must now also derive the `sov_modules_api::schemars::JsonSchema` trait.

## 2024-11-04
- #1800 Removed the `CliWalletArg` macro, removing support for parsing call messages directly as CLI arguments (for instance, in `sov-cli`). CLI utilities wishing to accept callmessages on the command line should accept a JSON string instead.
The `CliWalletArg` derives should be deleted on all module callmessages, as the macro no longer exists.

## 2024-11-01
- #1803 Changes the `InnerVm` and `OuterVm` types of `Spec` to a new `Zkvm` aggregate trait, and renames the old trait to `ZkVerifier`. It also replaces the old `Zkvm` bounds on `StateTransitionFunction` with the new trait. Finally, it removes the `InnerZkvmHost` and `OuterZkvmHost` bounds from `FullNodeBlueprint` and removes the `StateTransitionVerifier` type on `ParallelProverService`.
## 2024-11-01
 - #1801 Replaces usages of `String`s in `CallMessage`s with a new `SafeString` wrapper. This is used to enforce restrictions to ensure the strings are safe to display to users in a non-confusing way as part of a schema. A `SafeString` can be failibly constructed from a `String`.
Users wishing to continue using unconstrained Strings, despite the potential to affect security by confusing and obfuscating transaction contents when presented to the user during signing, are able to create their own newtype `String` wrapper and implement `SchemaGenerator` manually for it.

## 2024-10-31
- #1795 Adds a new `cors` field to the `runner.[rpc|axum]_config` section of the rollup configuration file, which can be used to disable CORS with `"disabled"`. `fn register_endpoints` now requires the entire `rollup_config` as a parameter.
- #1785 Changes the transaction signing interface. Now, a `chain_hash` value, derived from the rollup's generated universal schema, must be supplied on both signature and verification. This affects many testing interfaces, including any integration tests involving mock transaction construction; a mock hash value now has to be passed (and must be consistent between the test's transaction generation and subsequent verification).
A new parameter for this was also added to the transaction authentication capability. Rollups should implement schema generation according to the example in `demo-rollup`, and the schema's generated root hash value must be passed to the rollup runtime's authenticator. This value should then also be used in any full end-to-end test implementations.

## 2024-10-30
- #1786 Removes the `BondingService` associated type and `create_bonding_service` method on `FullNodeBlueprint`. These items can now be deleted.
## 2024-10-29
- #1778 ensures that the slot hooks only run when the visible slot number increases. This may be breaking behaviors that assume the slot hooks to run on every slot.

## 2024-10-28
- #1757 Adds two new parameters to the `genesis` method of the modules. Please ensure to reflect that change in your `genesis` implementations.

## 2024-10-25
- #1741 Update the `BatchReceipt` structure. Now the gas related information is tracked in `BatchSequencerReceipt`.

- #1748 adds a `tx_hash` field to the object format used by `/sequencer/events/ws`.


## 2024-10-24
- #1738 Adds a new field `gas_payer` to `StandardProvenRollupCapabilities`. To retain the previous behavior, use `self.bank` in this field. If you wish to opt into the new Paymaster module (which the sequncer to pay gas on behalf of users), add the  `sov_paymaster::Paymaster` module to your runtime and use it as the `gas_payer` instead.

## 2024-10-23
- #1730 Renames the `sov_modules_api::EnumUtils` function to `NestedEnumUtils` to better reflect its purpose.

## 2024-10-16

## 2024-10-18
- #1676: `SequencerRegistry`: the `minimal_bond` is determined by the batch size.
Consumers of the SDK should update their configuration files accordingly (`minimum_bond` was removed from the `sequencer_registry.json`).

## 2024-10-17
-  #1667: `stf-blueprint`: Extract the authentication logic into a separate stage. This change will break SDK consumers who rely on how the sequencer is penalized for processing invalid transactions.
- #1674 Increase users' balances on the rollup. Consumers of the SDK should update their configuration files accordingly.

## 2024-10-16
- #1665 Refactors and cleans-up the `Kernel` new capabilities. Consumers of the SDK may need to implement the `KernelSlotHooks` over the `Runtime` using the default implementation if that is not already the case.
- #1671 Adds `Borsh`, `SchemaGenerator`, `Arbitrary`,  bounds to Da layer address types. It also changes the `SequencerRegistry::CallMessage` type to use `Da::Address` instead of `Vec<u8>` where applicable.

## 2024-10-11
- #1636 Removes the `Kernel` structure from the `StfBlueprint`. It also adds and implements the `HasKernel` trait on the `Runtime`. Please make sure to remove the explicit dependence on the `Kernel` structure in your `StateTransitionFunction` implementations and use the `Runtime` instead. 

## 2024-10-14
- #1642 Accumulate the sequencer's rewards in its staking account. This change will impact SDK consumers who assumed the rewards were accumulated in the sequencer's `personal` account.
## 2024-10-15
- #1661 Adds `max_authentication_gas` to the `Authenticator` trait and corresponding `MAX_AUTHENTICATION_GAS` constant in `constants.toml`.
This is a breaking change and the consumers of the SDK have to add `MAX_AUTHENTICATION_GAS` to `constants.toml`
- #1660 Remove the slash_sequencer capability. This change is only relevant to the sequencers.
- #1663 Makes all existing `JsonSchema` trait requirements apply in the non-native mode. This allows removal of some inconvenient feature gates. 
## 2024-10-10
- #1607 Fixes querying future rollup height via REST API. Now it returns HTTP 404 instead of data at the rollup head.

## 2024-10-05
- #1624 Adds `DaSpec` as an associated type of `Spec` and removes it from every other type inside the module system. See the changes to the demo-rollup here for an example migration: https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/1624/files#diff-d9126f60816d820a29c0bf89e154c54c031f9fec4490301d08c3a3f2b39310e2
- #1610 requires a *direct* dependency on the `strum` crate for any packages that define a `Runtime` struct, or a dev-dependency on `strum` if the package uses `sov-test-utils` to generate a test runtime.
## 2024-10-05
- #1624 Adds `DaSpec` as an associated type of `Spec` and removes it from every other type inside the module system. See the changes to the demo-rollup here for an example migration: https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/1624/files#diff-d9126f60816d820a29c0bf89e154c54c031f9fec4490301d08c3a3f2b39310e2

- #1581 Fixes misalignment of rollup_height and JMT version. Genesis data is available at `rollup_height=0` via REST API.
## 2024-10-05
- #1619 Allows the Kernel information to be immediately available from the transaction context in the non-preferred sequencer mode. Users may experience breaking changes if they were relying on the previous behavior - ie the Kernel information written in the slot _i_ would only be available in the slot _i+1_.
- #1581 Fixes misalignment of rollup_height and JMT version. Genesis data is available at `rollup_height=0` via REST API.

## 2024-10-11

- #1630 allows customization of rollup address prefixes (also known as "HRP") by setting the `ADDRESS_PREFIX` constant in `constants.toml`. E.g.:
  
  ```toml
  [constants]
  ADDRESS_PREFIX = { const = "myrollup" }
  ```

## 2024-10-08
- #1599 removes the kernel macros and moves the Kernel modules inside the runtime. This may be a breaking change if users have implemented their own Kernels (we have removed the `Genesis` configuration from the `Kernel` trait) or if they have used the `Kernel` macro inside their modules (`KernelModule` derive macro, `kernel_module` attribute).

- The two type generics of `ApiState` have been inverted, e.g. `ApiState<Bank<S>, S>` is now `ApiState<S, Bank<S>>`, and the second generic defaults to `()`.
## 2024-10-08
- #1589 Revamp universal wallet generation code
This fully integrates the new `UniversalWallet` macro, which was previously present as beta versions. This macro should be derived on any types which may be part of the `RuntimeCall` or transaction types of a rollup; once derived, a schema for those types can be generated and used in rollup-agnostic wallets. The derivation should be gated by `#[cfg(feature = "native)]`.

For module authors, the `UniversalWallet` macro should be derived on the `CallMessage` type of your module(s), and subsequently any types used in the `CallMessage` (the compiler will enforce this). Any modules that already derived the previous implementation of the macro should double-check the import path: it is available at `sov_modules_api::macros::UniversalWallet`; the derivation and import must also be `"native"` feature-gated as above.

The underlying traits that the macro implements are also available to be used as members of the `sov_modules_api::sov_universal_wallet::*` module. The `SchemaGenerator` trait can be implemented manually as an alternative to deriving the macro, and the `OverrideSchema` trait can be used to manually designated a different type's implementation to be used for a given type - provided their `borsh` encodings are identical.

The macro is additionally available in `sov_rollup_interface` at the path `sov_rollup_interface::sov_universal_wallet::UniversalWallet`.

## 2024-10-07
- #1588 changes the signature of the `GasEnforcer` capability, adding one new method `try_reserve_gas_for_proof` and altering the signature of `try_reserve_gas` to include the entire `Context` instead of just the sender address. If you're using the standard capabilities, no change is needed. If you implemented your own `GasEnforcer` capability, simply copy-paste your old implementation as `try_reserve_gas_for_proof`, and use `context.sender()` in place of the `sender` argument within `try_reserve_gas`.
- #1584 extract transaction authentication to a separate methods.

## 2024-10-05
- #1581 Fixes misalignment of rollup_height and JMT version. Genesis data is available at `rollup_height=0` via REST API.

## 2024-10-04
- #1577 introduces a new `GasMeter::gas_info` method to the `GasMeter`. This is a breaking change for SDK consumers. Any code that previously used `gas_meter.gas_price()` will need to be updated to `gas_meter.gas_info().gas_price`, and similarly for `remaining_funds` and `gas_used`.

## 2024-10-03
- #1525 Adds `title` to `IntOrHash` enum variants in OpenAPI spec for LedgerAPI. This improves generated clients in some cases.
- #1568 `TransactionAuthorizer` capability no longer charges for the gas.
This is a breaking change for the consumers of the SDK. The capabilities accept 
`TxScratchpad` instead of `PreExecWorkingSet`.
- #1565 `GasEnforcer::try_reserve_gas` now accepts `TxScratchpad` instead of `PreExecWorkingSet`. This is a breaking change for the consumers of the SDK. 

## 2024-10-02
- #1557 Simplifies the internal representation of the `ChainState` module and ensures that `gas_price` updates can be immediately accessible through the `Kernel` interface. The gas price accessible from the kernels should now update immediately after a slot is processed. This can potentially break tests that rely on the previous behavior (ie, 1-slot delay for the gas update).
- #1547 Adds support for `DerivedHolder` in the `TokenHolder`. 
With this change token holder can be generated programmatically.

- #1549 introduces **an optional** `public_address` field to `runner_config`. 
   If this `public_address` is provided, it will be used as the server entry in the OpenAPI specification and UI. 
   This allows for the correct rendering of the server address in the OpenAPI spec when the rollup node is running behind a proxy.
   
With this change token holder can be generated programmatically. Example DerivedHolder: "derived_1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqgfk7l6"

- #1548 Moves OpenAPI response objects to schemas so it has name and client generators have better output.
  Adds missing 404 case for StateVecElementResponse

## 2024-10-01
- #1539 Removes JSON-RPC APIs from all modules, except `sov-evm` (note that JSON-RPC support for custom modules was **not** removed). **Users should switch to their REST API counterparts**. Additionally, code generated by `#[derive(HasRestApi)` no longer depends on `sov-state`, so the dependency can usually be removed.
- #1546 Removes `salt` from `CallMessage::CreateToken` in the bank module. This is are the breaking changes for the clients of the API:
1. The salt hast to be removed form all the call messages. 
2. The "GAS_TOKEN_ID" was updated from `token_1rwrh8gn2py0dl4vv65twgctmlwck6esm2as9dftumcw89kqqn3nqrduss6` to `token_1nyl0e0yweragfsatygt24zmd8jrr2vqtvdfptzjhxkguz2xxx3vs0y07u7` 

- #1544 `SequencerAuthorization::penalize_sequencer` capability now takes `TxScratchpad` as an argument.
 
## 2024-09-30
- #1537 Removes need to define swagger-ui in `Runtime::endpoints` implementation. Now it works out of the box by `sov_modules_rollup_blueprint::register_endpoints`.
- #1542 removes support for `config_bech32!`, and merges it into the main proc-macro `config_value!`.

  ```toml
  [constants]
  # ...

  # BEFORE (Rust: `config_bech32!("MY_CONST", TokenId)`)
  MY_CONST = "token_1qwqr2h2e5g961t4f2m1qt3t3d7xx7r4jchjc9ey5pe1r5u8ers9ts"

  # AFTER (Rust: `config_value!("MY_CONST")`)
  MY_CONST = { bech32 = "token_1qwqr2h2e5g961t4f2m1qt3t3d7xx7r4jchjc9ey5pe1r5u8ers9ts", type = "TokenId" }
  ```

  Moreover, the `config_value!` macro (1) is now not `const` expression -compatible by default and (2) supports overriding values in tests via env. variables. See the docs of `sov_modules_api::macros::config_value!` for a guide.

## 2024-09-26
- Changes the argument to the `Runtime::endpoints`, `HasRestApi::rest_api` and `get_rpc_methods` functions to `sov_modules_api::rest::ApiState<(), S>`
- Renames the `ApiStateAccessor::new` function to `from_storage` and adds the `Kernel` as a second argument.

- #1518 Removes the hardcoded `Vec<u64>` type to represents the gas price inside the `BatchReceipt` struct. The `StateTransitionFunction` trait now also has an associated `GasPrice` type to represent the notion of gas price inside `sov-rollup-interface`. Consumers of the SDK should ensure they replace their use of the `Vec<u64>` with a new generic `GasPrice` type and to specify the associated `GasPrice` type inside their implementations of the `StateTransitionFunction`. 

## 2024-09-25
- Adds new `JsonSchema` bounds on all `Address` types and `DaService::Config`. 
- Makes a significant refactor to the `BatchBuilder` trait
- Changes the signature of the `create_endpoints` function of `RollupBlueprint` to be `async
- Removes the third generic parameter from `RollupConfig` and changes the type of the second one from `DaService::Config` to `DaService`
- Renames the `sequencer_address` item in the `rollup_config.toml` to `da_address` to clarify the usage. 
- Changes the syntax of the `sequencer` section in `rollup_config.toml`. Now, callers must include one of `[sequencer.standard]` or `[sequencer.preferred]` there.


- #1510 Removes the `from_slice` method from the `GasArray` trait in favor of implementations of `From<[u64; config_value!("GAS_DIMENSIONS")]` and an implementation of `TryFrom<Vec<u64>>`. Consumers of the SDK should ensure they replace their use of the `from_slice` method with `From<[u64; config_value!("GAS_DIMENSIONS")]` (preferred solution, for type-safety reasons), otherwise they can use the `TryFrom<Vec<u64>>` method.
- #1514 Refactors `TestRunnerWithKernel::config.override_next_header_timestamp` to instead be `TestRunnerWithKernel::config.freeze_time`. It will now set the timestamp of all blocks to the provided timestamp instead of only the next block.
- #1509 Adds a `GasSpec` trait that is blanket implemented over the `Spec` trait. This adds a `GAS_DIMENSIONS` constant to be specified in the `constants.toml` file and removes all the other gas dimensions. Consumers of the SDK should ensure they only use one gas dimension and they specify the value of the `GAS_DIMENSIONS` constant in the `constants.toml` file.
- #1493 Makes it mandatory to manually set the `max_fee` in the CLI wallet. Transactions that don't set that value won't be accepted by the CLI. Consumers should ensure there is always a value set for the `max_fee` when using the CLI.

## 2024-09-20

Adds a new `arbitrary::Arbitrary` bound on the `Spec::Address` associated type and the `PrivateKeyExt` trait when `arbitrary` feature is enabled.

- #1470 Expresses all the bonds from the `AttesterIncentives`, the `ProverIncentives` and the `SequencerRegistry` in terms of multi-dimensional gas units instead of raw token value. That way the bond value of the rollup `sequencers/attesters/challengers/provers` will become dependent of the fluctuations of the gas price (which, in turn, makes the security of the rollup *independent* of the gas price fluctuations).
- #1480 Add the AttestationsManager, which is responsible for managing Attestation creation. This PR will break the `sov-rollup-starter` in several ways:
  1. Updates `prover_address` in rollup_config.toml.
  2. Adds a new method and an associated type to `FullNodeBlueprint`
- #1467 Adds the prefix `sov_` to all metrics emitted by the rollup.
- #1472 Export `ModuleRestApi` at the top level of `sov_modules_api` rather than `sov_modules_api::macros::ModuleRestApi`
- #1474 Add a test for the optimistic workflow
- #1463 Brings several fixes and changes in generated REST API for Runtime with modules:
   - Breaking: If StateValue or elements in StateVec and StateMap are not found HTTP 404 is returned instead of HTTP 200 with null value.
   - Correct return type for `StateValue` in OpenAPI spec.
   - Correct bech32 regex for `TokenId` and `ModuleId` in OpenAPI spec.
   - List of modules endpoint is brought back after accidental disabling.
   - Added `operationId` to custom REST API endpoints in Bank module.
- #1464 Improve the code structure of the integration tests
- #1461 Add `ProcessManager` that manages processes consuming StateTransitionInfo.
- #1459 Modifies the `TxState` trait to make it compatible with the `ApiStateAccessor` so that it becomes possible to test module call methods directly using the `ApiStateAccessor`.
-  #1457 Adds the `sov-stf-runner/processes` module and moves the `ProofManager`, `ProverService`, and `sov-stf-info-manager` there.
- #1456 Fixes infinite redirect in /swagger-ui endpoint.
- #1454 LedgerDb: use async_get to get data from the db.
- #1446 Integrate stf_info_manager in runner.rs
- #1450 Renames two related traits: `RuntimeAuthorization` becomes `TransactionAuthorizer`; `RuntimeAuthentiation` beomces `TransactionAuthenticator`. The `runtime_authorization` method on the `HasCapabilities` trait is also renamed to `transaction_authorizer`. 
- #1443 Move StateTransitionInfo channel creation to `FullNodeBlueprint`.
- #1442 Makes stf-runner handle trailing slashes without throwing HTTP 404. 
- #1438 enforces a consistent `snake_case` convention for JSON. **This requires updating the value `operating_mode` in `chain_state.json`** from `"Zk"` to `"zk"`. 
- #1436 Adds `get_operating_mode` to the `FullNodeBlueprint`. This tells the rollup whether it is running in zk or optimistic mode.
- #1432 stf_info_manager: Simplify the Db pruning logic
- #1428 Make stf_info_manager compatible with the StorageManager workflow.
- #1423 Adds Db pruning in the stf_info_manager.
- #1424 Breaking change in `DaService`. An associated type "`TransactionId`" is added to `DaSpec` and the return type of of `send_transaction` and `send_aggregated_zk_proof` has changed as follows:
  ```rust
  // New type:
  pub struct SubmitBlobReceipt<T: Debug + Clone> {
    // Canonically calculated blob hash
    pub blob_hash: HexHash,
    // DA native transaction id.
    pub transaction_id: T,
  }
  
  pub trait DaService {
     // rest is omitted...
     
     async fn send_transaction(
        &self,
        blob: &[u8],
        fee: Self::Fee,
     ) -> Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>;
  
     async fn send_aggregated_zk_proof(
        &self,
        aggregated_proof_data: &[u8],
        fee: Self::Fee,
    ) -> Result<SubmitBlobReceipt<<Self::Spec as DaSpec>::TransactionId>, Self::Error>;=  
  }
  ```
- #1416 Adds the correct version of the kernel state root to the `VisibleRoot`. We have to be careful to add the correct `kernel` state root to the `VisibleRoot` because the `historical_state_transitions` map that contains the roots is indexed by `true_rollup_height`. Hence, we have to make sure we get the `kernel_state_root` corresponding to the current `visible_height` accessible from the user space.
- #1418 Add back pressure mechanism to the StfInfoManager 
- #1409 Removes the `override_sequencer` field from test case structures and `TestRunner::execute` & `TestRunner::simulate` in favor of a single central location.
    - Instead of passing this param you should set `runner.config.sequencer_da_address` right before executing your test case. Also note that the semantics have changed, previously
      following test cases would revert back to the old sequencer da address - this is no longer the case. If you need this behavor you should save and restore the sequencer address.
      Example: `https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/1409/files#diff-2ce1f1e8ed6a6e93b23ddeee55eb317ff55dcf8a6fa1fb544916f3f67ee7b9abR257`
- #1415 Add StfInfoManager, which manages data related to state transitions. 
- #1413 Remove borsh bounds from StateTransitionWitness.
- #1407 Renames `transition_num` to `rollup_height` in the `AttesterIncentives` module.
- #1405 Fixes rendering of OpenAPI spec to match derived REST API endpoints for modules. 
- #1406 Removes the `Authenticator` trait and shifts responsibility for encoding authentication information into the `TransactionAuthenticator`. All
references to the `Authenticator` trait or `ModAuth` struct are replaced with references to `TransactionAuthenticator` or the concrete runtime type of the rollup as appropriate.
- #1398 Makes `StateCheckpoint`, `TxScratchpad` and `KernelStateAccessor` generic over `Storage` instead of `Spec`. These structs having no dependency on the `Spec`, this restricts the scope of the generics.
- #1397 Removes the DA generic from the `Kernel` trait.
- #1393 makes the new version of the kernel state accessors type-safe by preventing them to be built using the `From` trait of the `KernelStateAccessor`, but rather using a new `accessor` method in the `Kernel` trait. It also moves the `true_slot_height` out of the `StateCheckpoint` to the `KernelStateAccessor`
- #1378 Plugs in the new state accessors used in soft-confirmation. From now on, accessors such as the `StateCheckpoint` can access `VersionedStateValues` in the storage using the same mechanism as soft-confirmations.
- #1381 Adds an associated `Input` type the `TransactionAuthenticator` trait and expects that type as the argument to `authenticate`. It also adds a new method to the trait `fn encode_default_tx()` which implemetns the runtime-specific notion of a "standard" authentication path. 
- 1392 Simplify the test in prover/attester incentives.
- #1399 `AttesterIncentive`: Add balance checks to the challenger.rs tests.
- #1395 Add balance check to the `attestation_processing` test.
- #1392 Simplify the test in prover/attester incentives.
- #1388 Make the naming in the sov-test-utils more consistent.
- #1383 Extend `prover-incentoves::test_valid_proof` test to check gas usage. 
- #1370 Makes some light changes to the `GenesisStateAccessor` to become a wrapper around `StateCheckpoint`. 
- #1362 Adds a versioned state accessor to be used in the soft confirmation context. This versioned state accessor is append-only and should be initialized at genesis to be properly used.
- #1366 Replaces the `Delta` by a `StateCheckpoint` inside the `TxScratchpad`. This is because we are going to add fields to the `StateCheckpoint` (`visible_height`, `true_height`) - this will allow to propagate the values up to the `WorkingSet`.
- #1370 Makes some light changes to the `GenesisStateAccessor` to become a wrapper around `StateCheckpoint`. 
- #1356 Adds OpenAPI spec for sov-bank custom REST API endpoints.
- #1374 Add gas handling in the stf-blueprint::process_proof.
- #1377 Enables REST API endpoints for `AttesterIncentives` module.
- #1369 add an extra generic type to the `sov_evm::authenticate` function. This generic enables conversion between `reth::address` and `Spec::Address`. This change is only breaking for users of the EVM module.
- #1365 Simplify sequencer reward workflow.
- #1378 Plugs in the new state accessors used in soft-confirmation. From now on, accessors such as the `StateCheckpoint` can access `VersionedStateValues` in the storage using the same mechanism as soft-confirmations.
- #1362 Adds a versioned state accessor to be used in the soft confirmation context. This versioned state accessor is append-only and should be initialized at genesis to be properly used.
- #1366 Replaces the `Delta` by a `StateCheckpoint` inside the `TxScratchpad`. This is because we are going to add fields to the `StateCheckpoint` (`visible_height`, `true_height`) - this will allow to propagate the values up to the `WorkingSet`.
- #1360 attester-incentive & prover-incentives: Ensure that the helper methods only read the state.
- #1358 Adds a mechanism for individually overriding capabilities on the `HasCapabilities` trait.
    - All usages of `runtime.capabilities()` should updated to the capability name in `snake_case`.
      For example, `runtime.capabilities().try_reserve_gas` should become `runtime.gas_enforcer().try_reserve_gas`.
- #1344 Adds revertable errors to the `sov-attester-incentives` module.
- #1353 Removes the `Batch` argument from the `begin_batch_hook` to allow the preferred sequencer to process batches.
- #1352 adds a way to retrieve the `base_fee_per_gas` in the sequencer at the current visible height. To do that we added a method to the `ChainState` that returns the current `base_fee_per_gas` at the visible slot and a method to the `KernelSlotHooks` trait that allows easy access from the `Kernel`. It also removes the output from the `begin_slot` hook in the `KernelSlotHooks` trait, to allow more consistency accross the hooks (they shouldn't return anything).
- #1335 Removes the previously deprecated method `StateCheckpoint::to_working_set_deprecated`. If you still use this method, please migrate your code to the testing framework available in `sov_test_utils`.
- #1332 Remove events from AttesterIncentives capabilities 
- #1328 Add reverts support in `prover-incentives` module.
- #1329 Adds custom REST API endpoints to `sov-nft` module. Migrates `test-harness` to REST API, so `rpc_url` is not accepted anymore, and `rest_url` renamed to `node_url`. `genesis_dir` parameter is also removed, as data is sourced from REST API now. 
- #1322 Remove ProcessAttestation & ProcessChallenge call messages from the sov-attester-incentives.
- #1323 Modifies the `Event`, `DispatchCall`, `MessageCodec`, and `CliWallet` proc-macros to generate `enum`s with `PascalCase` instead of `snake_case`, as per Rust naming conventions.
- #1320 Updates challenger tests in attester-incentives module for invalid challenges.
- #1314 Updates challenger tests in attester-incentives module.
- #1314 Updates challenger tests in attester-incentives module.
- #1316 Renames `sov-nft-module` to `sov-nft`. Runtimes that use it need update `Cargo.toml` and runtime definition.
- #1312 Made changes to the module structure of the `sov-rollup-interface` crate. You'll now find full-node-related code and interfaces in `sov_rollup_interface::node`; we suggest using `cargo doc` to better navigate the new crate structure.
- #1306 Updates tests in attester-incentives part 2.
- #1276 Migrates sov-cli to using raw REST API requests. `rpc` subcommand replaced with `api`.
- #1308 Removes the associated type `FullNodeBlueprint::DaConfig` and moves it over to `DaService::Config`. This is most often used as the second generic of `RollupConfig`, which should become `RollupConfig<..., <Self::DaService as DaService>::Config, ...>`.
- #1299 Updates tests in attester-incentives
- #1275 Adds an `ExecutionContext` enum to `stf::apply_slot` and `Context::new`. This enum allows callees to distinguish between sequencer execution and normal "full node" execution.
- #1297 Renames the `?height=...` query parameter in the REST API to `?rollup_height=...`.
- #1290 Refactors slashing mechanism in the prover incentives module.
- #1287 Breaking Change: Adds `FromStr` trait bound to a `BlockHashTrait`. 
- #1291 New flag `--skip-if-present`: `sov-cli keys import --skip-if-present` allows to skip importing file if was previously imported.
- #1264 Makes testing-harness generic over runtime.
- #1267 Fixes HTTP 500 in Ledger REST API `slots/<number>` endpoint.
- #1261 Return Attestation and Challenge in ProofReceipt 
- #1252 Add `Attestation` data in the `ProofReceiptContents::Attestation` variant.
- #1245 Add `sov-attester-incentive` to the rollup's `Runtime`. Adds relevant config files.
- #1238 Adds a custom REST API endpoint to bank module, allowing query balance. Fixes `ModuleRestApi` derive macro to correctly pickup specialized implementation of `HasCustomRestApi` trait.
- #1234 Add support for `process_attestation & process_challenge` capabilities in the `STF blueprint`.
- #1233 Add support for the optimistic workflow in the `STF blueprint`.
- #1228 Converts sov-attester-incentive to capability.
- #1223 Adds logging of response times for celestia adapter. Minimal configuration: `RUST_LOG="info,sov_celestia_adapter::da_service=trace,jsonrpsee=trace,jsonrpsee-http=info,jsonrpsee-client=info"`
- #1227 Changes public API in `StateDb`. `StateDb::materialize_preimages` and `StateDb::materialize_node_batches` are not generic anymore and now take 2 parameters for `kernel` and `user` namespace data.
- #1199 renames `sov_state::EncodeKeyLike` to `sov_state::EncodeLike`, so that it can also be used for values when interfacing with `StateValue`, `StateMap`, and `StateVec`.
- #1169 removes the `#[serialization]` attribute consumed by the `DispatchCall` and `event` macros. Now, the enums generated by those macros derive `borsh` and `serde` by default. Use `#[event(no_default_attrs)]` and `#[dispatch_call(no_default_attrs)]` to opt out of the defaults. Use `#[event({attr})]` or `#[dispatch_call({attr})]`to add an attribute to the generated enums.
- #1172 adds a new configuration section `[sequencer.batch_builder]` to the rollup configuration TOML file. Inside this section, you must specify the sequencer DA address. This was previously set in the `[da]` section as `celestia_own_address`.
- #1073 Changes error handling in `demo-rollup`, so it not panics on error, but high level main function prints error and exits with appropriate code
- #1141 Renames `risc0-cycle-macros` to `sov-cycle-macros` and changes the usage pattern. Callers should typically import the `sov-cycle-utils` crate and use its `macros` module rather than importing `sov-cycle-macros` directly.
- #1109 removes the sequencer tx status notifications `published` and `finalized`, and replaces them with `processed` instead. A `processed` notification instructs the client that the ledger has processed the transaction and it's ready to serve finality data, its receipt, and its events.
- #1146 Adds `DEFAULT_SOV_ROLLUP_LOGGING` constant in `sov-modules-rollup-blueprint` that should be used for default tracing-subscriber setup.
- #1143 Adds new reth crate and removes copy pasted code. `jsonrpsee` and `tokio` have been updated.
- #1149 Adds `FinalizedBlocksBulkFetcher` to stf runner for faster sync.
- #1121 changes the `BatchReceipt`'s `inner` default field and adds the `sequencer_da_address` to it. In particular:
    - Add a new `BatchSequencerReceipt` struct which contains the sequencer's `da_address` and `BatchSequencerOutcome`.
    - Change the `BatchResult` type from `BatchSequencerOutcome` to `BatchSequencerReceipt`.
    - Remove the sender's `Da::Address` field from the `end_batch_hook` because it is now accessible from `BatchResult`
    - Minor stylistic improvements in the `StfBlueprint`.
- #1120 Improve error handling in SequencerRegistry.
- #1135 Removes need for a `secp256k1` patch for risc0, as `k256` crate is used in guest context. Also switches to upstream reth from Sovereign fork
- #1129 changes `StateVec::iter` to return `Result`s to account for state reading errors. The new method `StateVec::collect_infallible` can be used in tests and for infallible state accessors.
- #1108 improve error handling in `ProverIncentives` module.
- #1104 Add `deposit` method to prover incentives module.
- #1106 Upgrades [`reth`](https://github.com/paradigmxyz/reth/) dependencies to `1.0.3`. Still based on Sovereign fork.
- #1003 Removes the `BlobData` enum, requiring sequencers to post the `Batch` struct directly on chain.
- #1076 turns the `FullNoteBlueprint::create_endpoints` and `register_endpoints` functions async. It also modifies the sequencer to emit a new tx status event when transactions are dropped from the mempool.
- #1062 Adds `remove` method to `StateVec`
- #978 Upgrades rust toolchain to 1.79.
- #1047 Use ProverIncentive module in the StfBlueprint for proof verification.
- #1025 Pass the `genesis_state_root` to the `ProverService`
- #1024 removes the associate type `DaService::TransactionId` and replaces it with `BlobReaderTrait::BlobHash`. The type signatures of `DaService::send_aggregated_zk_proof` and `DaService::send_transaction` now return `BlobHash` instead of the unit type.
- #1024 removes the associated type `DaService::TransactionId` and replaces it with `BlobReaderTrait::BlobHash`. The type signatures of `DaService::send_aggregated_zk_proof` and `DaService::send_transaction` now return `BlobHash` instead of the unit type.
- #1021 Moves the `BatchSequencerOutcome` type to the `sov-modules-api` crate.
- #1011 Rename `hooks.rs` to `capabilities.rs` in the `Accounts` module and add `capabilities.rs` for `ProverIncentives` module.
- #1004 Slash the sequencer if the aggregated proof can't be deserialized in the proof processing workflow.
- #1007 adds `slotNumber` to batch objects and `batchNumber` to transaction objects returned by the ledger API.
- #1001 StfBlueprint cleanup: Remove ApplyBatch type.
- #995  Move `SequencerRegistry::hooks` logic to `SequencerRemuneration` capability.
- #1062 Adds `remove` method to `StateVec`
- #966 Adds gas & fees relevant logic to the `STF::process_proof` method.
- #954 Replaces `ProverStorageManager` with `NativeStorageManager`. `StateDb`, `AccessoryDb` and `LedgerDb` now have different constructors.
- #956 Split stf_blueprint into smaller chunks.
- #950 Add metadata about gas & fees to the serialized proof.
- #943 Add `ProofSerializer` trait which allow adding additional metadata to the proof blob.
- #908 refactors the testing framework to achieve the following objectives:
   - Allow the execution of transactions from different modules in one test
   - Allow the execution of multiple batches within one slot
- #927 Refactor common fields in `Transaction`, `UnsignedTransaction` & `Message` into a new `TxDetails` struct.
- #913 Read prover address from file configuration file.
- #910 replaces `constants.json` with `constants.toml` and `constants.test.json` with `constants.testing.toml`. The expected file "schema" and contents are the same, just the file format has changed and you'll just need to translate your file contents from JSON to TOML.
- #906 Simplifies the `UnmeteredStateWrapper` wrapper struct and adds it in the testing framework as a wrapper of the `TxState` trait to prevent test maintainers from charging gas in the hooks.
-  #889 Add `rewarded_addresses` filed to `openapi::AggregatedProofPublicData` 
-  #869 Add prover address to `AggregatedProofPublicData` 
- #904 removes the need to specify event keys when calling `self.emit_event`, because the event key is now generated automatically based on the module name and the event variant (e.g. `Bank/TokenCreated`). You can use `self.emit_event_with_custom_key` to emit events with custom keys.
- #900 Track & charge pre-execution gas costs during direct sequencer registration.
- #881 adds blessed constants in the `constants.toml` to be used to charge gas in the tests. It also removes an unused method `tx_fixed_cost` that got deprecated with the recent changes in gas. The default value of the `MAX_FEE` in the CLI wallet is now `10_000_000`.
- #864 Add prover address to `AggregatedProofPublicData` 
- #885 renames the `rpc.rs` file in the standard SDK module template to `query.rs`.
- #869 Add prover address to `AggregatedProofPublicData` 
- #862 Add `prover_address` to `StateTransitionPublicData` this will allow rewarding the prover.
- #859 Adds a `MeteredSignature` struct wrapper and a `MeteredBorshDeserialize` trait that respectively charges gas for signature verification and borsh deserialization. We have renamed the `hash.rs` file to `metered_utils.rs` inside the `sov-modules-api/common` crate and grouped all the custom metered utils there. 
- #863 Upgrades borsh to version 1.0. If you import borsh in one of your crates, be sure to upgrade as well.
- #856 removes support for the `ledger_*` RPC methods and replaces them with a REST API accessible by default at `http://localhost:12346/ledger`.
- #862 Adds `StateTransitionWitnessWithAddress` struct that is used to pass the prover address to the Prover.
- #853 Make the `ProverIncentives` call methods public and accept prover address explicitly.
- #823 Adds StorableMockDaService, where blocks will be persisted between restarts. `[da]` section for adapter has been changed. 
- #821 removes support for the RPC methods of rollup sequencers, and replaces them with a REST API accessible by default at `http://localhost:12346/sequencer`.
- #851 Add BlobData constructors. 
- #842 `ProofManager` saves zk-proofs received from the `STF` in the db. 
- #839 Refine "ProofProcessor" capability.
- 835 adds a new `MeteredHasher` struct that charges gas for every hash computation. This structure is meant to be used in the module system to charge gas when hashing data.
- #835 adds a new `MeteredHasher` struct that charges gas for every hash computation. This structure is meant to be used in the module system to charge gas when hashing data.
- #828 add `ProofProcessor` capability.
- #813 is a follow-up of #783, it sets the constants that define the costs for gas access to non-zero values. That way, we charge some gas when trying to access the storage and the metered accessors (like the `WorkingSet`) can now run out of gas because of state accesses.
Meaningful changes:
  - We had to change the blessed values for the transaction fee, the initial account balances and the attester/prover stakes. The new values are: `MAX_FEE = 1_000_000`, `INITIAL_BALANCE = 1_000_000_000` and `DEFAULT_STAKE = 100_000`.
  - We had to replace the gas computation of some tests that made strong assumptions about the gas consumed by transaction execution.
- #814 mandates `0x` prefixes for hashes in RPC and REST APIs, whereas before it was optional.
- #809 extend blob-storage to support proof DA namespace.
- #806 simplify new_test_blob_from_batch
- #783 enables gas metering for storage accesses inside the module system. In particular, it makes all the state accessors fallible, and the module code now have to handle the case where the state accessor runs out of gas.
Meaningful changes
  - Making all the state accessors fallible: for instance `state_value.get(&mut state_reader)` now returns `Result<Option<StateValue>, StateReader::Error>`. Depending on the type of the state reader, the error type may be `Infallible` (for unmetered state accessors) or `StateAccessorError` (for metered state accessors)
  - The metered state accessors are: `WorkingSet` and `PreExecWorkingSet` when accessing the provable state; the unmetered state accessors are `TxScratchpad`, `StateCheckpoint`, `KernelStateAccessor`, ... 
  - A summary of all the access control patterns can be found in `sov-modules-api/src/state/accessors/access_controls.rs`
  - A new cargo dependency has been added: `unwrap_infallible`. This allows us to safely unwrap the type `Error<T, Infallible>` which has become ubiquitous because the `Infallible` error type is raised whenever an unmetered state accessor tries to access the state (416 occurrences in our codebase after this PR is merged).
  - The `WorkingSet` use, specially its instantiation with the `WorkingSet::new` pattern has been *significantly* reduced in favor of using the `StateCheckpoint` in the tests. Since the `StateCheckpoint` is an unmetered state accessor, this makes the tests more manageable and maintainable - we can use `unwrap_infallible`. Besides, one now needs to use a special `GenesisStateAccessor` to instantiate the modules at genesis, which needs to be built from the `StateCheckpoint`.
  - temporary `evm` feature that allows converting the `WorkingSet` to an `UnmeteredStateWrapper`. **This is a temporary solution and that feature will be removed once we find a way to connect the EVM and module gas metering**. This is the final task of https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/734.

- #800 `LedgerDb` explicitly returns changes instead of saving them in local cache of underlying `CacheDb`.
- #796 Reuse Batch struct in BatchWithId & PreferredBatch.
- #789 Changes the Transaction receipt type to include the reason a transaction was skipped or reverted. Consumers of the API should now use the `sov_stf_blueprint::TxEffect` as the `TxReceipt` return type for ledger RPC queries.
- #790 Simplify `BlobSelector` code.
- #753 updates default `max_fee` in `sov-cli` to `10_000` (from previous value of `0`)
- #791 adds `Self::TransactionId` to `DaService::send_aggregated_zk_proof`.
- #787 split capabilities into separate modules.
- #770 enforces that transactions are set with the correct chain ID. To get the ID, use `config_value!("CHAIN_ID")`.

- #775 adds a couple of custom error types that will be useful to allow using the `try` pattern within the module system to automatically convert errors into `anyhow::Error`. Meaningful changes:
  - Adding a custom `GasMeteringError` enum that describes the possible causes for error inside the `GasMeter` trait methods
  - Adding a custom `StateAccessorError` enum that describes the situations where accessing the state might fail.
  - Adding the `std::error::Error + Send + Sync` bounds to the associated `Error` type in `StateReader` and `StateWriter` (this allows automatic conversion to `anyhow::Error`
- #766 modifies the RPC interface to accept an `ApiStateAccessor` instead of a `WorkingSet` to prepare the full integration of the gas metering for state accesses. In particular this commit changes the `RPC` macro to accept an `ApiStateAccessor` instead of a `WorkingSet` as an argument to the rpc methods.
- #726 adds Swagger UI -an OpenApi playground- as an endpoint defined inside `sov_modules_stf_blueprint::Runtime::endpoint`.
- #743 adds metered state accessor traits to the `sov-modules-api/state` module.
- #764 changes notifications logic, so ledger notifications arrive after state has been completely updated.
- #750 moves `TransactionAuthenticator, TransactionAuthorizer, and Authenticator` to a separate file in the capabilities module.
- #751 adds `DaServiceWithRetries` wrapper.
   - To use it, make sure your `DaService::Error` type is `MaybeRetryable<anyhow::Error>`. You can then wrap your `DaService` in `DaServiceWithRetries` whilst providing a policy for exponential retries via: `DaServiceWithRetries::with_exponential_backoff(<da-service>, <exponential-backoff-policy>)`. This wrapped `DaService` can then be used normally as a `DaService` anywhere.
   - Use the `MaybeRetryable` return type in your `DaService` to indicate whether a fallibe function maybe retried or not, by returning either `MaybeRetryable::Transient(e)` if you want your functions to be retried in the case of errors, or `MaybeRetryable::Permanent(e)` if you don't want them to be retried.
- #749 makes the rollup generic over the `AuthorizationData` type.
- #742 moves the scratchpad and the state into a submodule of the sov-modules-api and breaks these files into smaller components to make the complexity more manageable.
- #730 Splits `RollupBlueprint` into two traits: `FullNodeBlueprint` and `RollupBlueprint` and feature gates the full-node blueprint behind the `"native"` feature flag. It also reduces the number of required types for the `RollupBlueprint` by making it generic over execution mode. See the diff of `celestia_rollup.rs` for a complete example of a migration.
- #735 updates `minter_address` to `mint_to_address` in `CallMessage::CreateToken` & `CallMessage::Mint`
- #725 removes the `macros` feature from `sov-modules-api`, which is now always enabled even with `--no-default-features`.
- #732 adds the `sov-nonces` module.
- #714 integrates a batch of changes to the `StfBlueprint` and the capabilities. Meaningful changes:
  - Remove arguments of type `SequencerStakeMeter` from the capabilities and replace all of the `(SequencerStakeMeter, StateCheckpoint)` couple of variables by a single `PreExecWorkingSet` which is a type safe data structure that should charge for gas before transaction execution starts.
  - Removing the `ExecutionMode` type from the `StfBlueprint`. It is now replaced by `TxScratchpad`, which is an intermediary type between `WorkingSet` and `StateCheckpoint`. This is useful for the `sov-sequencer` crate because one may want to revert all the changes that happened during transaction execution when simulating a transaction before adding it to a sequencer batch.
  - The entire transaction processing lifecycle is now embedded in the `process_tx` method that is called by both the `apply_batch` in `StfBlueprint` and `try_add_tx_to_batch` in the `sov-sequencer` crate. Hence we now have the following state machine for transaction execution:
        - In `to_tx_scratchpad` we convert the `StateCheckpoint` into a revertable `TxScratchpad` for transaction execution.
        - In `authorize_sequencer` we consume the `TxScratchpad` and build a `PreExecWorkingSet` on success, we stop processing the batch on failure
        - In `try_reserve_gas` we consume the `PreExecWorkingSet` and we build a `WorkingSet` on success (transaction execution starts), or return a `PreExecWorkingSet` on failure (so that we can penalize the sequencer).
        - In `attempt_tx` we consume the `WorkingSet` and output a `TxScratchpad`
        - The `penalize_sequencer` method consumes a `PreExecWorkingSet` and outputs a `TxScratchpad`
  - The `SequencerTxOutcome` type is removed. Now all pre-execution capabilities are processed in `process_tx`. When entering `apply_tx`, the sequencer cannot be penalized anymore.

- #699 adds tests for the `EVM` credentials.
- #717 replaces manual serialization & deserialization for the signed parts of `Transaction` struct by creating a new `UnsignedTransaction` object that implements Borsh traits. The breaking change is that the bytes over we sign now include an additional vector length for the runtime message field.
- #700 adds an associated `TxState` type to the `TxHooks` trait and uses it as an argument in place of a concrete `WorkingSet`.
- #563 introduces a REST API for modules and runtimes. The `RollupBlueprint::create_endpoints` method in `demo-rollup` has been updated accordingly, so it exposes both JSON-RPC and the REST API by default. You can find the documentation for generating a REST API in `sov_modules_api::rest`.
- #696 requires `Runtime` implementers to implement the `HasCapabilities` trait, which allows the `Runtime` to delegate to another struct for much of its required functionality. If a `Runtime` does not wish to delegate, it can simply implement the trait with `Self` as the associated `Capabilities` type. Implementations can be found in the sov-capabailities crate.
- #701 adds support for multiple credentials in `sov-accounts`. This is a breaking change for the consumers of the SDK.
- #694 adds `Credentials` to the `Context` structure. This is a breaking change for the consumers of the SDK. See implementation of the `TransactionAuthorizer::resolve_context` method.
- #688 follow-up of https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/681 that modifies the gas interface to prevent access to `TxGasMeter` outside of `sov-modules-api`.
The spirit of this change is to increase the coupling between the `WorkingSet` and `Transaction` types. Meaningful changes:
  - Change the visibility of `TxGasMeter` to `pub(crate)`.
  - Change the `WorkingSet` interface to return `TransactionConsumption` instead of `GasMeter` when consumed with `revert` or `checkpoint`.
  - Move the `TransactionConsumption` and `SequencerReward` types to `scratchpad`
  - Change the `GasEnforcer` capability to handle the gas workflow without having access to `TxGasMeter`. In particular change the `consume_gas_and_allocate_rewards` capability to the simpler `allocate_consumed_gas` which distributes the transaction reward returned when calling `checkpoint` on the `working_set` to the base fee and tip recipient.
  - Remove access to methods that can artificially modify the gas meter outside of testing (like methods to build `unmetered` gas meters).

- #683 replace `Public Key Hash` with more general concept of `CredentialId`. This is a breaking change for the consumers of the SDK.
- #689 makes `#[rpc_gen]` annotations on modules optional, so you can safely remove it if you don't need it. Additionally, the `get_rpc_methods` function generated by `#[expose_rpc]` now doesn't depend on the import of e.g. `BankRpcImpl` and you should these imports.
- #683 replace `Public Key Hash` with more general concept of `CredentialId`. This is a breaking change for the consumers of the SDK.
- #686 Changes the API of `Module::genesis` to accept an `&mut impl sov_modules_api::GenesisState<S>` instead of a concrete type.
- #679 remove the transaction signature check from the `EVM` module.
- #681 contains some interface improvements for the gas which become possible after `sov-core` and `sov-api` got merged. In particular:
    - Move `TxGasMeter` from `common/gas` to `transaction` which allows more type-safety by removing methods to create and modify arbitrary gas meters, tying it to the `AuthenticatedTransactionData` type.
    - Remove `Tx` and `TxGasMeter` associated types from the `GasEnforcer` trait.
- #647 completes the gas workflow for the StfBlueprint by enhancing the interface by adding some type-safety guarantees to the StfBlueprint and simplifying the penalization workflow. Follow-up of #619.
    - Added a new `consume_gas_and_allocate_rewards` capability to the GasEnforcer to allocate transaction rewards at the end of the transaction execution. This was previously done in `refund_remaining_gas`
    - Added a new type `TransactionConsumption` that tracks the amount of gas consumed and can only be built using the `AuthenticatedTransactionData` and by consuming the associated `TxGasMeter`.
    - `refund_remaining_gas` can only be called either after `consume_gas_and_allocate_rewards` or with a zero `TransactionConsumption` (speculative case for reverted transactions)
    - after `consume_gas_and_allocate_rewards` the `GasMeter` is consumed and cannot be used anymore
- #673 Removes `std` feature from `rollup-interface` and `no_std` support. usage of `sov_rollup_interface::maybestd` should be changed back to `std`.
- #680 Extends sov-cli:
    - Adds new optional boolean parameter to `submit-batch`, that tells sov-cli to wait for batch to be processed by full node
    - set url now expects second parameter for REST API endpoint.
- #663 Modifies the interface of traits `TransactionAuthenticator` and `TransactionAuthorizer`. Associated types `Tx` and `Gas` have been removed. `TransactionAuthenticator` is now generic over `S: Spec`. Methods' type signatures have been slightly modified; please see `examples/demo-rollup/stf/src/authentication.rs` for an example on the new usage.
- #633 Deprecate `sov-modules-core`, move definitions into `sov-modules-api` & `sov-state`
- #664 removes the `Transaction` wrapping in `sov-ethereum` for EVM transactions. This is a breaking change for consumers of the SDK. See `TransactionAuthenticator::authenticate`.
- #646 adds authenticator dispatch logic in`TransactionAuthenticator::authenticate`.
- #613 Makes `sov_state::Storage` trait to be immutable and explicitly produce changes. SimpleStorageManager should be used when data needs to be persisted between batches.
- #631 removes the need for modules to `#[derive(ModuleCallJsonSchema)]`; the trait is automatically blanket-implemented for all modules as long as `CallMessage` implements `schemars::JsonSchema`.
- #628 all the account resolution logic was moved to `resolve_context`. This method now returns a `Result<Context, _ >` instead of a `Context`. This is a breaking change for consumers of the SDK.
- #621 removes the need for a prelude `sov_modules_api::prelude` which re-exposes a few common types for convenience, as well as external crates like `clap` and `serde_json` (for now, more will follow). You can remove these dependencies from your `Cargo.toml` if you wish.
- #620 Adds more fields to the `Event`s emitted by the `sov-bank` module. Start emitting events for token minting.
- #619 starts charging gas for signature checks in the StfBlueprint and completes the refactoring effort started in #612. There was the following changes in the interface:
  - Introduction of a `GasMeter` trait and the three associated implementations: `TxGasMeter` (what used to be the `GasMeter` struct), `UnlimitedGasMeter` (a gas meter that holds an infinite reserve of gas) and the `SequencerStakeMeter` (a gas meter specially designed to track the sequencer stake and accumulate penalties).
  - Adding the `sequencer_stake_meter` as an argument of the `authenticate` method of the `TransactionAuthenticator` (as an associated type) and the `Authenticator` (as a `&mut impl GasMeter` in that case).
  - Adding the `refund_sequencer` capability which can be used to refund the sequencer some of the penalties he accumulated during the pre-execution checks.
  - Modify the `authorize_sequencer` and `penalize_sequencer` capabilities to take the `SequencerGasMeter` as a parameter instead of a fixed amount. This allows type safety and removes the unsafe `saturating_sub` in the implementation of `penalize_sequencer`.
  - Add a `TxSequencerOutcome` which is an enum with the variants `Rewarded(amount)` and `Penalized`.
  - Rename `SequencerOutcome` to `BatchSequencerOutcome` to represent the `SequencerOutcome` following batch execution.

- #622 Make `DefaultStorageSpec` generic over a `Hasher` instead of defaulting to `Sha256`
- #622 Make `DefaultStorageSpec` generic over a `Hasher` instead of defaulting to `Sha256`
- #623 updates the AccountConfig structure. This change is breaking for consumers of the SDK, as the format of the accounts.json configuration files has been changed.
- #617 adds a `gas_estimate` method to the DA layer `Fee` trait
- #614 Change the final argument of `Module::call` from a concrete `WorkingSet` to an abstract `impl TxState` type. It also removes the `working_set.accessory_state()` method and grants direct access to that state through the `WorkingSet` (read/write) and `impl TxState` (read-only).
- #612 refactors the `StfBlueprint` to deserialize, perform signature checks and execute transactions in one pass instead of doing pre-checks for the entire batch followed by the batch execution. Simplifies and fix the sequencer reward workflow. In particular:
  - Replace the previous mechanism that was using a mutable `i64` typed, `sequencer_reward` variable that accumulated the reward/penalties for the sequencer in the entire batch. This mechanism was buggy and did not properly account for side effects (e.g when the sequencer gets penalized more than his current stake, his transactions shouldn't be processed anymore).
  - Augment the authorization error for the `rawTx` verification with an enum with 2 variants: `FatalError` (sequencer gets slashed for acting maliciously) and `Invalid` (sequencer gets penalized a constant amount from his balance).
  - Remove the `Penalized` variant from the `SequencerOutcome`. This is replaced by the `penalize_sequencer` capability that is called whenever the sequencer gets penalized.
  - Add a `authorize_sequencer` capability that verifies that the sequencer has locked enough tokens to execute the next transaction. Since the sequencer bond can vary between transactions (because he may get penalized), this method needs to be called whenever the sequencer start executing a new transaction.
- #599 changes the way you define Rust constants which use the `constants.toml` file.
  Instead of the attribute macro `#[config_constant]`, you'll now use `config_value!`, like this:

  ```rust
  // Before
  #[config_constant]
  pub(crate) const GAS_TX_FIXED_COST: [u64; 2];

  // After
  pub(crate) const GAS_TX_FIXED_COST: [u64; 2] = config_value!("GAS_TX_FIXED_COST");

  // For Bech32:
  // Before
  #[config_bech32_constant]
  const TEST_TOKEN_ID: TokenId;

  // After
  const TEST_TOKEN_ID: TokenId = config_bech32!("TEST_TOKEN_ID", TokenId);
  ```

  Read the PR description for more details.

- #586 Adds a second Zkvm generic to the `StateTransitionFunction` API. This VM is used for generation of *`Aggregate`* zk proofs,
while the first VM continues Ato be used for block production. The signature of the `StfVerifier<DA, Vm, ZkSpec, RT, K>` was also changed to `StfVerifier<DA ZkSpec, RT, K, InnerVm, OuterVm>`

- #584 removes support for the `DefaultRuntime` derive macro. You must replace all proc-macro invocations of `DefaultRuntime` with `#[derive(Default)]`.
- #590 upgrade rustc version to 1.77. Installation new risc0 toolchain is needed. Simply
  run `make install-risc0-toolchain`
- #584 removes support for the `DefaultRuntime` derive macro. You must replace all proc-macro invocations
  of `DefaultRuntime` with `#[derive(Default)]`.

- #572 adds a new `da_service` method `estimate_fee()` and requires all blob submission methods to provide a `fee` as an argument.
- #580 `sov-cli` now returns an error on duplicate nickname key import or generation.
- #572 adds a new `da_service` method `estimate_fee()` and requires all blob submission methods to provide a `fee` as an
  argument.
- #573 Changes the `Bank::create_token signature`, allowing modules to create tokens.

- #547 removes the `From<[u8;32]>` bound on rollup addresses and replaces it with a bound expressing that for
  any `Spec`, the `Address` type must implement `From<&PublicKey>`. It also removes the nft module's `CollectionAddress`
  type and replaces it with a 32-byte `CollectionId`.
- #546 fixes a bug where sequencer could protect themselves from slashing by exiting while their batch was being
  processed.

- #545 moves the `RelevantBlobs`, `RelevantBlobIters`, and `DaProof` types from `rollup_interfaces::da::services`
  to `rollup_interfaces::da`. It also adds a feature gate to require the `native` flag to
  use `rollup_interfaces::da::services`.

- #535 fixes the sequencer tip calculation in the `StfBlueprint`. This calculation now takes into account the changes of
  the #429 (which fixes the gas reserve calculation). Also renames `max_priority_fee` to `max_priority_fee_bips` in
  the `Transaction` structure.

- #528 removes the `initial_base_fee_per_gas` parameter from the genesis configuration of the chain state to define a
  constant `INITIAL_BASE_FEE_PER_GAS` that is common to all crates. Now that we assume the tx sender always has a bank
  account for the gas token, the non-zero transfer amount hacks from the `reserve_gas` and `refund_remaining_gas`
  capabilities for the `GasEnforcer` in the `Bank` module are replaced by an account check. If the sender doesn't have a
  bank account for the gas token, the method fails with the error `AccountDoesNotExist`. This check is done in the
  EIP-1559 specification.
- #519 Adds an `Authenticator` trait which abstracts away the transaction authentication logic. This is a breaking
  change so the consumers of the SDK will need to implement the new trait.
  See `authentication.rs` in `demo-rollup`.

- #538 removes the chain state from the EVM module because it was unused.

- #528 removes the `initial_base_fee_per_gas` parameter from the genesis configuration of the chain state to define a
  constant `INITIAL_BASE_FEE_PER_GAS` that is common to all crates. Now that we assume the tx sender always has a bank
  account for the gas token, the non-zero transfer amount hacks from the `reserve_gas` and `refund_remaining_gas`
  capabilities for the `GasEnforcer` in the `Bank` module are replaced by an account check. If the sender doesn't have a
  bank account for the gas token, the method fails with the error `AccountDoesNotExist`. This check is done in the
  EIP-1559 specification.

- #526 `Bank::create_token` & `Bank::mint` now accept an `impl Payable` as arguments. This is a breaking
  change. `&S::Address` already implements `Payable`, and `ModuleId` can be promoted to `Payable`
  via `ModuleId::as_token_holder`.

- #525 mitigates the bug where a transaction which was included multiple times would always return the `Duplicate` tx
  status rather than returning info about the original tx execution. In this commit we use a simple heuristic (first tx
  wins) to guess which instance is the "correct" one.


- #468 fixes the gas elasticity computation that was removed in #476. The base fee computation is now done in
  the `ChainState` module as part of the `begin_slot` hook. This PR also updates the `ChainState` integration tests to
  check that the base fee computation is correctly performed in the module hooks. It also adds gas elasticity tests for
  the `ChainState` module.

- #490 Adds a `inner_code_commitment`, an `outer_code_commitment` and a `initial_da_height` field to the `ChainState`
  module. These fields should be initialized at genesis. Added getters for `genesis_da_height`, `outer_code_commitment`
  and `inner_code_commitment` in the `ChainState` module. Adapted the `demo-rollup` json configurations to use these
  fields. Added a new configuration folder for the stf tests that rely on `MockCodeCommitment` which have a different
  format from the `risc0` code commitments (32 bytes instead of 8). Modified the `AttesterIncentives` module to use
  the `inner_code_commitment` field from the `ChainState` module instead of
  the `commitment_to_allowed_challenger_method` field. Modified the `ProverIncentives` module to use
  the `outer_code_commitment` field from the `ChainState` module instead of the `commitment_of_allowed_verifier_method`
  field.

- #487 Introduces the `AuthenticatedTransactionData` structure. This is the transaction data that passed the
  authentication phase. And updates `Accounts::CallMessage` format. This is a breaking change for consumers of the SDK
  only if they send messages directly to the Accounts module.

- #484 Adds a new `CodeCommitment` trait and applies it to the associated type of the ZKVM. The
  trait provides encode/decode methods, in addition to all existing functionality. These methods
  should be used to convert to/from the `code_commitment` vector in `AggregatedProofPublicData`.

- #480 The `Accounts` module now keeps PublicKey hashes instead of PublicKeys. This is a breaking change for consumers
  of the SDK only if they send messages directly to the Accounts module.

- #479 refactors the `ChainState` module integration test to be more readable and less repetitive.

- #476 updates the gas interface for the ChainState module, removes the gas price elasticity computation (it will be
  fixed in #468) and propagates these changes throughout the infrastructure.
  Meaningful changes:
    - Added the `INITIAL_GAS_LIMIT` and `initial_gas_price` (defined at genesis) constants. These constants are defined
      in the EIP-1559 and are used to handle the gas lifecycle in the chain-state module
    - Rename the `gas_price` (a generic name not used anywhere in the EIP-1559) to `base_fee_per_gas` which is the
      official naming for this variable
    - Create a `BlockGasInfo` structure that groups the `gas_used`, `gas_limit` and `base_fee_per_gas` into one wrapper.
    - Removed the `gas_price_state` from the `chain-state` module's state. There was multiple reasons behind that:
    - Removed the outdated gas elasticity mechanism

- #481 This PR combines the `ContextResolver` and `TransactionDeduplicator` traits into a single `TransactionAuthorizer`
  trait. This is a breaking change, and consumers of the SDK will need to implement the new trait.

- #472 This PR breaks downstream code in the following way:
  `PublicKey::to_address` is now parameterized by `Hasher`.

- #471 adds 3 new parameters to sov-demo-rollup
    - optional cmd `--genesis-config-dir ../test-data/genesis/demo/celestia` to specify the genesis config directory
    - optional cmd `--prometheus_exporter_bind 127.0.0.1:9845` to specify the prometheus exporter bind address. Useful
      for running several nodes on the same host.
    - environment `export SOV_TX_SIGNER_PRIV_KEY_PATH=examples/test-data/keys/tx_signer_private_key.json` to specify the
      path to the transaction signer private key.

- #452 abstracts away the transaction authorization logic. The consumers of the `sov-module-api` have to implement the
  new `TransactionAuthenticator` trait. Refer to `hooks_impl.rs` for details

- #413 introduces new RESTful JSON APIs for the sequencer and, most importantly, modifies the `RollupBlueprint` trait
  interface to allow implementations to expose Axum servers, instead of only JSON-RPC servers. In
  fact, `RollupBlueprint::create_rpc_methods` was renamed to `RollupBlueprint::create_endpoints`, which returns a tuple.
  Most `RollupBlueprint` implementations will need to use the new `sov_modules_rollup_blueprint::register_endpoints`,
  which replaces `sov_modules_rollup_blueprint::register_rpc`. Take a look at how `examples/demo-rollup` implements the
  new interface to see how it works.

- #439 Implements the `SequencerRegistry` module to support Sequencers' reward and penalties. In particular,
  the `SequencerRegistry` can now be used in conjunction with the `GasEnforcer` capability hook to reward the
  sequencer for submitting a correct transaction.

- #444 Moves the tests for the `SequencerRegistry` module to the `src` directory of the same crate.

- #443 Removes the `coins` field in the `SequencerRegistry` struct. It is replaced by a `minimum_bond` field and
  the `TokenId` becomes `GAS_TOKEN_ID`. The configuration structure `SequencerRegistryConfig` should be updated to
  replace the `coin` field by the new `minimum_bond` field.

- #432 Updates the `StateTransitionFunction`  to handle blobs from all the relevant namespaces.
  This breaks the `StateTransitionFunction` API but the breaking changes don't propagate outside of the module system
  internals.

- #441 Removes the section of the `rollup_cofing.toml` called `[prover_service]` and moves its existing value to a
  section called `[proof_manager]`. To update, it's sufficient to simply search and replace `[prover_service]`
  to `[proof_manager]` in any configuration files.

- #429 Updates the `reserve_gas` and `refund_remaining_gas` mechanisms to match EIP-1559. The `reserve_gas`
  and `refund_remaining_gas` methods are moved back to the `Bank` module as they now affect multiple modules (the module
  that locks the gas tip - ie the `sequencer-registry` - and the module that locks the base gas - ie
  the `attester-incentives` or `prover-incentives`). Instead of locking the gas in
  the `attester-incentives`, `prover-incentives` or `sequencer-registry` at the `reserve_gas` call, we are now doing it
  when `refund_remaining_gas` is called. The `Transaction` structure is updated to let the user specify a `max_fee` and
  a `max_priority_fee` which are respectively a coin amount and a percentage. He may optionally specify a `gas_limit`
  which is a multi-dimensional gas limit that is used as a protection for gas elasticity (following EIP-1559).

- #425 Updates the `CelestiaVerifier` to support multiple namespaces. This change is breaking for consumers of
  the `Sovereign-sdk`: The `CelestiaVerifier` now needs to be initialized with `ROLLUP_BATCH_NAMESPACE`
  and `ROLLUP_PROOF_NAMESPACE`. See:
    1. https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/425/files#diff-75e27b2869f342897e1c89ed4abe7ff82ce8368a795dbefdffac8e30bbcb11f4L36

    2. https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/425/files#diff-d46bdfc6e8e6dfb4acd9794c4536d6a8212b37aef27abc4b39d7db479be75d4aL135

- #406 Updates the `DaService` trait and the `Celestia` adapter to support multiple namespaces. This changes are
  transparent to the `RollupBlueprint`.

- #361 starts charging gas for submitting transactions to the Rollup. When calling `apply_slot`, the transaction sender
  must pay for a fixed amount of gas - `GAS_TX_FIXED_COST`. Developers have to make sure the transaction sender has
  enough funds to pay for the gas.

- #385 makes the `reward_burn_rate` constant in the `ProverIncentives` module and transforms the associated getters to
  be infallible. In the future, the reward burn rate will have to be set in the `constants.toml` and
  the `constants.testing.toml` files and need to be a value in the range [0, 99].

- #340 moves the Kernels' implementation (currently the `BasicKernel` and the `SoftConfirmationsKernel`) to a
  dedicated `sov-kernel` crate.

- #347 renames the following types:
  `StateTransitionData` to `StateTransitionWitness`
  `StateTransition` to `StateTransitionPublicData`
  `AggregatedProofPublicInput` to `AggregatedProofPublicData`

- #329 adds `InnerZkvm` and `OuterZkvm` associated types to the `Spec` trait.

- #306 removes the `State*Accessor` traits and replaces them with methods on (Acessory)StateValue/Map types. You can
  simply remove
  any imports of these traits and the `sov_modules_api::prelude*`. Also simplifies the API of VersionedStateValue. Now
  it only has a method `get_current` (for any type implementing the `VersionReader` trait)
  and get/set implemented directly on `KernelStateAccessor`

- #266 implements reward/slashing mechanisms for provers in the `ProverIncentives` module. In particular, given that an
  aggregated proof can be correctly serialized and the proof outputs are corrects, the provers can be rewarded for the
  new block transitions they proved. If no new block transitions are proved as part of the aggregated proof, then the
  prover is penalized by a fixed amount.
  The prover may be slashed if it posts an invalid proof or a proof for a state transition that doesn't exist.

- #170 unifies `CacheKey/Value` and `StorageKey/Value` data structures into `SlotKey/Value` data structures.

- #253 adds block validity conditions as part of aggregated proofs public inputs. This then can ensure that the validity
  conditions are stored on-chain for out-of-circuit verification. The validity conditions are stored as a `Vec<u8>`,
  after being serialized using `Borsh`.

- #242 changes the behavior of the `AttesterIncentives` module to gracefully exit when users are slashed and the state
  gets updated. The slashing reason can be retrieved as part of the `UserSlashed` event that gets emitted. Also contains
  small changes to the traits derived by the structures contained in the module, so that the module can be included in
  the runtime structures. We also add the `Checker` associated type to the `DaSpec` trait which considerably simplifies
  the module structure definition (contains two generics instead of 4)

- #169 achieves the rollup state separation in different namespaces. Conceptually, each namespace is just defined by a
  triple of tables inside a shared state db - there is only one `StateDb`.
- #451 Removes optional transactions list from RPC endpoints `eth_publishBatch`.
