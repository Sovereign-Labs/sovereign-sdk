# sp1-microbenches

Microbenchmark harness for calibrating the ZK dimension of `constants.toml` gas
constants. Runs isolated SP1 guest programs through `ProverClient::execute()` to
capture **prover gas** as a function of input size, then fits a linear cost
model `cost = bias + per_byte × input_size`.

## Why prover gas, not RISC-V cycles?

SP1 splits execution across multiple specialised proving chips: one for
ordinary CPU instructions, plus a separate chip per precompile (SHA-256,
Keccak, secp256k1, etc.). The RISC-V cycle counter increments once per guest
instruction regardless of which chip ends up doing the work, so it
underrepresents precompile work — a SHA-256 syscall is "1 cycle" in the
counter but triggers a whole block-compression's worth of work in the
precompile chip's trace. Calibrating against cycle counts would undercharge
precompile-heavy operations and overcharge ordinary-instruction operations.

`ExecutionReport.gas()` is SP1's own model of proving cost, trained against
real GPU proving times. It looks at trace shape across every chip and
correlates with the resource we actually want to charge for. The harness
prints raw per-row cycle counts as an eyeball sanity check (you can see at a
glance whether the precompile is active by the cycles/byte rate), but the
fitted constants come from prover gas.

[SP1 docs on prover gas.](https://docs.succinct.xyz/docs/sp1/optimizing-programs/prover-gas)

## What's being measured

Each guest mirrors the production verify call site rather than the raw
primitive.

**SHA-256** — the guest invokes
`MeteredHasher::<UnlimitedGasMeter<S>, S::CryptoSpec::Hasher>::digest` — same
code path as `calculate_hash_metered` in
`crates/module-system/sov-modules-api/src/runtime/capabilities/authentication.rs`.
Results calibrate `GAS_TO_CHARGE_HASH_UPDATE` and
`GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE`.

**Ed25519** — the guest invokes `SP1Signature::verify` directly (uses
`ed25519_consensus`, SP1-patched, routes through the curve precompile). This
matches `verify_signature_unmetered` in
`crates/module-system/sov-modules-api/src/transaction/mod.rs:285` — the same
primitive call that the metered path eventually makes. Results calibrate
`DEFAULT_FIXED_GAS_TO_CHARGE_PER_SIGNATURE_VERIFICATION` and
`DEFAULT_GAS_TO_CHARGE_PER_BYTE_SIGNATURE_VERIFICATION`.

**Celestia verifier** — the host loads the existing Mocha fixture
`crates/adapters/celestia/test_data/block_mocha_multi_candidate_rows_10261831`,
builds the same relevant blobs and proofs used by production verification, and
sends the serialized filtered block plus verifier inputs to the SP1 guest. The
guest reads and deserializes the full block, runs
`CelestiaVerifier::verify_relevant_tx_list`, and commits block/hash/blob stats.
This is a fixed fixture bench rather than a byte-size sweep.

## Run

```sh
cargo run --release -p sp1-microbenches -- sha256
cargo run --release -p sp1-microbenches -- ed25519
cargo run --release -p sp1-microbenches -- celestia
```

Optional override:

```sh
cargo run --release -p sp1-microbenches -- sha256 --iterations 2000
cargo run --release -p sp1-microbenches -- ed25519 --iterations 500
```

To skip the SP1 guest build (CI without the SP1 toolchain installed):

```sh
SKIP_GUEST_BUILD=1 cargo build --release -p sp1-microbenches
```

## Output

Results are printed to stdout in three blocks:

1. **Raw measurements** — one row per swept input size, with prover gas
   (total and per-iter), total cycles, region cycles (per-iter and total).
2. **Linear fit** — `bias`, `per_byte`, R², max residual.
3. **Suggested `constants.toml` values** — the fit values rounded to integers.

Values are in **raw SP1 prover-gas units**, intended to be wired into the ZK
gas dimension of `constants.toml` 1:1 — no scaling multiplier. We'd revisit
that decision only if a concrete reason to renormalise comes up (sub-unit
costs, numeric-range conflict with EIP-1559 math, etc.).

If we eventually want structured output (markdown report, CSV, JSON, etc.),
add it then — for now stdout is enough.

## Architecture

Each bench is a module under `src/cmd/`:

- `cmd/<name>.rs` defines `<Name>Args` (the CLI flags) and a
  `run(args) -> anyhow::Result<()>` function that runs the sweep, fits the
  data, and prints results.
- `cmd/mod.rs` is just a list of `pub mod <name>;` declarations.

The CLI parser and `BenchCmd` subcommand enum live in `src/main.rs` (the
binary) — they're pure clap-derive glue with no library role.

Shared infrastructure lives in `lib.rs` (`BenchResult`, `load_guest_elf`) and
`fit.rs` (OLS fit).

## Adding a new microbench

1. Create `guest-{name}/` mirroring `guest-sha256/` — same SP1 patches, same
   `[workspace]` detachment. Use `core::hint::black_box` around inputs and
   results in the hot loop to keep the optimiser from hoisting loop-invariant
   work (cheap insurance — see `guest-sha256/src/main.rs`).
2. Add a `build_program_with_args` call in `build.rs`.
3. Create `src/cmd/{name}.rs` with `{Name}Args` and
   `pub fn run(args: {Name}Args) -> anyhow::Result<()>`. Inside, run the
   sweep, call `fit_prover_gas_per_byte`, and `println!` the results.
4. Add `pub mod {name};` to `src/cmd/mod.rs`.
5. Add the variant to `BenchCmd` in `src/main.rs` and route it in
   `BenchCmd::run`.
