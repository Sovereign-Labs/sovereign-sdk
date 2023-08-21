# Directories and paths
TRACER_DIR = ../../utils/zk-cycle-utils/tracer
ELF_PATH_TRACER = ../../../examples/demo-prover/target/riscv-guest/riscv32im-risc0-zkvm-elf/release/rollup
TRACE_PATH_TRACER = ../../../examples/demo-prover/host/rollup.trace

# This allows you to pass additional flags when you call `make run-tracer`.
# For example: `make run-tracer ADDITIONAL_FLAGS="--some-flag"`
ADDITIONAL_FLAGS ?=

.PHONY: generate-files run-tracer

all: generate-files run-tracer

generate-files:
	ROLLUP_TRACE=rollup.trace cargo bench --bench prover_bench --features bench

run-tracer:
	@cd $(TRACER_DIR) && \
	cargo run --release -- --no-raw-counts --rollup-elf $(ELF_PATH_TRACER) --rollup-trace $(TRACE_PATH_TRACER) $(ADDITIONAL_FLAGS)
