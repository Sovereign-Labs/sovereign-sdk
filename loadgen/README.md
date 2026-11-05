# Sovereign loadr plugin

Native data-source plugin for the preferred sequencer. It fetches `/rollup/schema` once, signs transactions using that schema and its active chain hash, and lets loadr send the HTTP requests. This is a separate Cargo workspace.

Each VU owns one funded key. Each iteration queries that key's next nonce, then submits one signed transaction to `/sequencer/txs`. There is no optimistic nonce counter: a rejected submission or lost response is followed by a fresh lookup. This costs one GET per transfer. The plan checks `receipt.result == "successful"` so reverted transactions fail the test.

## Build and install

Use the loadr revision pinned in `Cargo.toml` on the controller and every agent. Older versions may lack VU placement or result callbacks. The plugin refuses initialization without placement information.

```sh
SDK_ROOT=/absolute/path/to/sovereign-sdk-public
cargo install --git https://github.com/celestiaorg/loadr --rev 8982d8c709c98f94dd1df9c919e651b670cdafb1 --locked loadr-cli
SKIP_GUEST_BUILD=1 cargo build --release --locked --manifest-path "$SDK_ROOT/loadgen/Cargo.toml"
mkdir -p /tmp/sov-bank
cp "$SDK_ROOT/loadgen/plugin.toml" "$SDK_ROOT/loadgen/target/release/libsov_loadr.so" /tmp/sov-bank/
loadr plugin install /tmp/sov-bank
```

The manifest above targets Linux. On macOS, use the `.dylib` artifact and change `entry` in the staged manifest.

## Configure and run

Copy `bank.yaml`, then edit `data.tx.config`:

| Field | Meaning |
| --- | --- |
| `url` | Preferred sequencer's REST base URL. |
| `keys_path` | Absolute path to a JSON array of 32-byte private keys encoded as hex, optionally prefixed with `0x`. Ed25519 keys are seeds, not 64-byte keypairs. |
| `crypto` | `ed25519` or `secp256k1`, matching the rollup's standard authenticator. |
| `max_fee` | Maximum transaction fee, as a decimal string. |
| `runtime_call` | Runtime-specific call JSON. The example sends a bank transfer; replace its recipient and token ID. |

For the demo runtime, the recipient is `{"Standard":"sov1..."}`. For an Ethereum-address runtime such as rollup-starter, use `crypto: secp256k1` and a recipient string such as `"0x..."`. Runtime module positions and address encoding come from the target schema. Both current versioned signing payloads and legacy `UnsignedTransaction` schemas are supported; the latter use `chain_id` and append the chain hash to the unsigned bytes.

Provision the accounts with both the transfer token and gas funds before running. The plugin does not create or fund accounts. A key file looks like `["<64 hex characters>", "<64 hex characters>"]`. Duplicate keys are rejected.

Use the same ordered key file on every agent. Agent VU `n` uses key index `vu_offset + n - 1`. Supply enough keys for the fleet's maximum VU allocation, including `max_vus`, and keep those accounts exclusive to this run. Use one scenario, one plugin source, and sequential requests named exactly `nonce` and `transfer`; do not put them in parallel flows.

```sh
loadr run /absolute/path/to/bank.yaml
```

The example schedules 100 transfers/s for 30 seconds with up to 32 VUs. This is a starting setting, not a throughput guarantee. For a smoke test, change the executor to `per-vu-iterations`, set `vus: 1`, `iterations: 5`, `max_duration: 30s`, and remove the arrival-rate fields. Increase rate and VUs after that succeeds. HTTP request counts include nonce lookups; use the `transfer` request metrics and `transfer executed` check for transaction results. Restart the run after a chain upgrade so it fetches the new schema and chain hash.

## Tests

```sh
SKIP_GUEST_BUILD=1 cargo nextest run --manifest-path "$SDK_ROOT/loadgen/Cargo.toml" -p sov-loadr
SOV_LOADR_STARTER_SCHEMA=/absolute/path/to/starter-schema.json SKIP_GUEST_BUILD=1 cargo nextest run --manifest-path "$SDK_ROOT/loadgen/Cargo.toml" -p sov-loadr --run-ignored only legacy_starter_schema
```

The optional starter test accepts the raw legacy schema JSON, without the REST response wrapper. It verifies serialization and the secp256k1 signature; it does not run a rollup.

After building the plugin, the other optional test runs loadr against a local HTTP endpoint that verifies every signature and nonce. Point `SOV_LOADR_BIN` to the matching loadr binary and `SOV_LOADR_PLUGIN` to the built shared library:

```sh
SOV_LOADR_BIN=/absolute/path/to/loadr SOV_LOADR_PLUGIN="$SDK_ROOT/loadgen/target/release/libsov_loadr.so" SKIP_GUEST_BUILD=1 cargo nextest run --manifest-path "$SDK_ROOT/loadgen/Cargo.toml" -p sov-loadr --run-ignored only loadr_executes_plan
```
