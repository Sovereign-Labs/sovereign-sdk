# sp1-microbenches

Microbenchmark harness for calibrating the ZK dimension of `constants.toml` gas
constants. Runs isolated SP1 guest programs through `ProverClient::execute()` to
capture **prover gas** and **RISC-V cycles** as a function of input size, then
fits a linear cost model.

## Why prover gas?

`ExecutionReport.gas()` predicts GPU proving time better than raw cycle counts —
keccak256 with 5.6M cycles can prove faster than ECDSA recovery at 4.4M cycles
because of differing precompile shard shapes. Prover gas is whole-execution-
only, so each microbench's program does one operation many times in isolation.

## What's being measured

Each guest mirrors the production charging call site rather than the raw primitive.
For SHA-256 the guest invokes `MeteredHasher::<UnlimitedGasMeter<S>, S::CryptoSpec::Hasher>::digest`
— same code path as `calculate_hash_metered` in
`crates/module-system/sov-modules-api/src/runtime/capabilities/authentication.rs`. The
results therefore calibrate the constants that the SDK actually consults
(`GAS_TO_CHARGE_HASH_UPDATE`, `GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE`), and apply only to
API-level hashing — JMT internal-node hashing uses the raw `S::Hasher` and is not
governed by these constants.

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

To skip the SP1 guest build (CI without SP1 toolchain):

```sh
SKIP_GUEST_BUILD=1 cargo build --release -p sp1-microbenches
```

## Output

A markdown report under `reports/` containing:

1. Methodology + environment (SP1 version, git commit, host).
2. Raw measurements per input size.
3. Linear fit `cost = bias + per_byte × input_size` with R² and max residual.
4. Suggested raw values for `GAS_TO_CHARGE_HASH_UPDATE[1]` and
   `GAS_TO_CHARGE_PER_BYTE_HASH_UPDATE[1]`.

The values are reported in **raw prover gas units**. Wiring them into
`constants.toml` requires a global scaling decision so all metered primitives
(hashing, signatures, deserialization, …) share a single gas unit.

## Adding a new microbench

1. Create `guest-{name}/` mirroring `guest-sha256/`.
2. Add a `build_program_with_args` call in `build.rs`.
3. Add a CLI subcommand and a `run_{name}` function in `src/main.rs`.
