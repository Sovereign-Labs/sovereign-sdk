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
correlates with the resource we actually want to charge for. We display raw
per-row cycle counts in the report as an eyeball sanity check (you can see at
a glance whether the precompile is active by the cycles/byte rate), but the
fitted constants come from prover gas.

## What's being measured

Each guest mirrors the production charging call site rather than the raw
primitive. For SHA-256 the guest invokes
`MeteredHasher::<UnlimitedGasMeter<S>, S::CryptoSpec::Hasher>::digest` — same
code path as `calculate_hash_metered` in
`crates/module-system/sov-modules-api/src/runtime/capabilities/authentication.rs`.
Results therefore calibrate the constants the SDK actually consults
(`GAS_TO_CHARGE_HASH_UPDATE`, `GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE`).

These constants apply only to API-level hashing — JMT internal-node hashing
uses the raw `S::Hasher` and is not governed by them; its cost is absorbed by
the storage-access gas constants instead.

## Run

```sh
cargo run --release -p sp1-microbenches -- sha256
```

Optional overrides:

```sh
cargo run --release -p sp1-microbenches -- sha256 \
    --iterations 2000 \
    --out reports/sha256-custom.md
```

To skip the SP1 guest build (CI without the SP1 toolchain installed):

```sh
SKIP_GUEST_BUILD=1 cargo build --release -p sp1-microbenches
```

## Output

A markdown report under `reports/` containing:

1. Methodology + environment (date, SP1 SDK version).
2. Raw measurements per input size (prover gas, cycles, both per-iter and
   total).
3. Linear fit `cost = bias + per_byte × input_size` with R² and max residual.
4. Bench-specific scope statement (what these constants do and don't cover).
5. Suggested raw values for the relevant `constants.toml` entries.
6. Glossary defining every term used in the report.

Values are reported in **raw SP1 prover-gas units**. We wire them into the ZK
gas dimension of `constants.toml` 1:1, without a scaling multiplier — prover
gas is already calibrated to be comparable across primitives and sits in a
comfortable numeric range for `u64` block-gas-limit arithmetic. We'd revisit
that decision only if a concrete reason to renormalise comes up (sub-unit
costs, numeric-range conflict with EIP-1559 math, etc.).

## Architecture

Each bench is a module under `src/cmd/`:

- `cmd/<name>.rs` defines its own `<Name>Args` (the CLI flags), a zero-sized
  `<Name>Bench` struct implementing the `ReportContent` trait (algorithm name,
  scope text, suggested-constants section, bench-specific glossary entries),
  and a `run(args) -> anyhow::Result<BenchOutput>` function.
- `cmd/mod.rs` enumerates the available benches in `BenchCmd` and dispatches
  them. The dispatcher also owns output-path resolution and file writing — the
  per-bench `run` function just produces a `BenchOutput { markdown,
  default_filename, summary }` and hands it back.

Shared infrastructure lives in `lib.rs` (`BenchResult`, `load_guest_elf`,
`today`), `fit.rs` (OLS fit), and `reports.rs` (markdown rendering + shared
glossary core).

## Adding a new microbench

1. Create `guest-{name}/` mirroring `guest-sha256/` — same SP1 patches, same
   `[workspace]` detachment. Use `core::hint::black_box` around inputs and
   results in the hot loop to keep the optimiser from hoisting loop-invariant
   work (cheap insurance — see `guest-sha256/src/main.rs`).
2. Add a `build_program_with_args` call in `build.rs`.
3. Create `src/cmd/{name}.rs` with `{Name}Args`, `{Name}Bench` (impl
   `ReportContent`), and `pub fn run(args: {Name}Args) -> anyhow::Result<BenchOutput>`.
4. Add the variant to `BenchCmd` in `src/cmd/mod.rs` and route it in
   `BenchCmd::run` and `BenchCmd::out`.
