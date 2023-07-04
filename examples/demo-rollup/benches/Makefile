# Default values for num blocks and transactions per block
BLOCKS ?= 100
TXNS_PER_BLOCK ?= 10000

export BLOCKS
export TXNS_PER_BLOCK

criterion:
	@echo "Running criterion bench with $(TXNS_PER_BLOCK) transactions per block"
	@echo "Method: Criterion"
	@echo "Output: Criterion"
	@cd .. && cargo bench --bench rollup_bench

basic:
	@echo "Running basic benchmark with $(BLOCKS) blocks and $(TXNS_PER_BLOCK) transactions per block"
	@echo "Method: Coarse Timers"
	@echo "Output: Standard"
	@cd .. && cargo bench --bench rollup_coarse_measure

prometheus:
	@echo "Running basic benchmark with $(BLOCKS) blocks and $(TXNS_PER_BLOCK) transactions per block"
	@echo "Method: Coarse Timers"
	@echo "Output: Prometheus"
	@cd .. && PROMETHEUS=1 cargo bench --bench rollup_coarse_measure

flamegraph:
	@echo "Running basic benchmark with $(BLOCKS) blocks and $(TXNS_PER_BLOCK) transactions per block"
	@echo "Method: Coarse Timers"
	@echo "Output: Flamegraph"
	@cd .. && sudo BLOCKS=$(BLOCKS) TXNS_PER_BLOCK=$(TXNS_PER_BLOCK) cargo flamegraph -o benches/flamegraph.svg --bench rollup_coarse_measure && sudo rm -rf benches/demo_data
