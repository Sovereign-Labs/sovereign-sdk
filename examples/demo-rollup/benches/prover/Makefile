# Default values for num blocks and transactions per block
BLOCKS ?= 10
TXNS_PER_BLOCK ?= 100
NUM_PUB_KEYS ?= 10000
GENESIS_CONFIG_DIR ?= ../test-data/genesis/benchmark

export BLOCKS
export TXNS_PER_BLOCK
export NUM_PUB_KEYS
export GENESIS_CONFIG_DIR

small_bench:
	@echo "Small benchmark configuration:"
	@BLOCKS=2 TXNS_PER_BLOCK=10 NUM_PUB_KEYS=10 make standard_bench

realistic_bench:
	@echo "Realistic benchmark configuration:"
	@BLOCKS=100 TXNS_PER_BLOCK=1000 NUM_PUB_KEYS=1000000 make standard_bench

standard_bench:
	@echo "Running benchmark with $(BLOCKS) transaction blocks, $(TXNS_PER_BLOCK) transactions per block, and $(NUM_PUB_KEYS) public keys"
	@cd ../.. && cargo bench --features="bench" --bench=prover_bench