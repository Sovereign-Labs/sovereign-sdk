# Verify Example Configs

Verify that example configuration files in `examples/demo-rollup/configs/` are complete and accurate compared to their Rust struct definitions.

## Instructions

### Step 1: List Config Files

Get committed config files only:
```bash
git ls-files 'examples/demo-rollup/configs/*.toml'
```

### Step 2: For Each Config File

#### 2.1 Detect DA Type

Read the `[da]` section to determine which DA adapter is used:
- Has `connection_string` field → **MockDa** (use `crates/adapters/mock-da/src/config.rs`)
- Has `rpc_url` field → **Celestia** (use `crates/adapters/celestia/src/config.rs`)
- Has only `url` field → **ExternalMock** (use `crates/adapters/mock-da/src/storable/rpc/client.rs`)

#### 2.2 Read Rust Struct Files

Read these files to extract field definitions:

| TOML Section | Rust File | Struct |
|--------------|-----------|--------|
| `[da]` | (depends on DA type above) | `MockDaConfig` / `CelestiaConfig` / `MockDaClientConfig` |
| `[storage]` | `crates/full-node/sov-db/src/config.rs` | `RollupDbConfig` |
| `[runner]` | `crates/full-node/full-node-configs/src/runner.rs` | `RunnerConfig` |
| `[runner.http_config]` | `crates/full-node/full-node-configs/src/runner.rs` | `HttpServerConfig` |
| `[monitoring]` | `crates/full-node/sov-metrics/src/influxdb/config.rs` | `MonitoringConfig` |
| `[proof_manager]` | `crates/full-node/full-node-configs/src/runner.rs` | `ProofManagerConfig` |
| `[sequencer]` | `crates/full-node/full-node-configs/src/sequencer.rs` | `SequencerConfig` |
| `[sequencer.standard]` | `crates/full-node/full-node-configs/src/sequencer.rs` | `StdSequencerConfig` |
| `[sequencer.preferred]` | `crates/full-node/full-node-configs/src/sequencer.rs` | `PreferredSequencerConfig` |
| `[sequencer.preferred.postgres_config]` | `crates/full-node/full-node-configs/src/sequencer.rs` | `PostgresConfig` |
| `[sequencer.preferred.rate_limiter]` | `crates/full-node/full-node-configs/src/sequencer.rs` | `SovRateLimiterConfig` |
| `[sequencer.extension]` | `crates/full-node/full-node-configs/src/sequencer.rs` | `SeqConfigExtension` |

#### 2.3 Extract Field Info from Rust

For each struct, extract:
- **Field name** (the identifier after `pub`)
- **Serde rename** if present: `#[serde(rename = "...")]`
- **Doc comment** (lines starting with `///` above the field)
- **Default value** from `#[serde(default = "...")]` or `#[serde(default)]`
- **Whether optional** (`Option<T>` type)

### Step 3: Verification Checks

#### 3.1 All Fields Referenced (only for `mock_rollup_config.toml`)

For the mock config only, verify every struct field appears in the TOML file either:
- As an active field: `field_name = value`
- As a commented field: `# field_name = value`

Report any fields that are completely missing.

#### 3.2 Comment Accuracy (all configs)

For each field in the TOML that has an inline comment (text after `#`), compare it to the Rust doc comment. Report if they differ significantly.

#### 3.3 Default Values (all configs)

For commented-out fields showing a default value like `# field = default_value`, verify the value matches what's defined in Rust via `#[serde(default)]` or the type's `Default` implementation.

### Step 4: Report Findings

Print a summary for each config file:
- List of missing fields (for mock_rollup_config.toml)
- List of comment mismatches with expected vs actual
- List of incorrect default values

If all checks pass, print: "All config files verified successfully."

### Step 5: Fix Issues (with permission)

If issues are found, ask the user:
"Found X issues in Y config files. Would you like me to update the configs?"

Only make changes if the user approves.

## Notes

- Focus on struct fields that map to TOML keys (ignore internal fields like `#[serde(skip)]`)
- Handle nested structs by matching TOML table paths (e.g., `[runner.http_config]` → `HttpServerConfig`)
- Fields with `#[serde(flatten)]` merge into the parent TOML section
- Enum variants like `SequencerKindConfig::Preferred` become `[sequencer.preferred]` in TOML
