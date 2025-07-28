# Contains INFLUX_ORG_ID and INFLUX_AUTH_TOKEN.
-include config.env
export

# Can be "verysmall", "small", "standard" or "large"
SIZE ?= very-small

SEED ?= 0

SUBFOLDER ?= $(shell date "+%Y-%m-%d")

# Should be greater than 0. Usually below 30.
NUM_THREADS_LOGS ?= 20

NUM_THREADS ?= 10

NUM_SLOTS ?= 100

DOCKER_PATH ?= ../../../docker

LOCALHOST ?= 127.0.0.1

TARGET_DIR := $(shell \
    dir=$$(pwd); \
    while [ "$$dir" != "/" ]; do \
        if [ -d "$$dir/target" ]; then \
            echo "$$dir/target"; \
            exit 0; \
        fi; \
        dir=$$(dirname "$$dir"); \
    done; \
)

start_metrics_container:
	@echo "Starting metrics container. Telegraf port: $(TELEGRAF_PORT), Influx port: $(INFLUX_PORT)..."
	@sudo docker compose --env-file config.env -f $(DOCKER_PATH)/docker-compose.obs.yml up -d --remove-orphans

stop_metrics_container:
	@echo "Stopping metrics container..."
	@sudo docker compose --env-file config.env -f $(DOCKER_PATH)/docker-compose.obs.yml down

restart_metrics_container: stop_metrics_container start_metrics_container

generate_benches:
	@echo "Generating $(SIZE) benchmarks with $(NUM_SLOTS) slots and seed $(SEED)..."
	@echo "Benchmark files stored at path: ./src/bench_files/$(SUBFOLDER)"
	@mkdir -p ./src/bench_files/$(SUBFOLDER)
	@cargo build -r --bin bench_generator --target-dir $(TARGET_DIR)
	@$(TARGET_DIR)/release/bench_generator -s $(NUM_SLOTS) --seed $(SEED) -p ./src/bench_files/$(SUBFOLDER) $(SIZE)

bench_runner:
	@echo "Running benchmark files path: ./src/bench_files/$(SUBFOLDER)"
	@mkdir -p ./src/metrics/$(SUBFOLDER)
	@cargo build -r --bin bench_runner --target-dir $(TARGET_DIR)

run_benches: bench_runner
	@output=$$(ls -t $(TARGET_DIR)/release/bench_runner | head -n 1) && \
	$$output -t $(NUM_THREADS) -p ./src/bench_files/$(SUBFOLDER)

run_benches_with_logs: bench_runner
	@echo "Logging threads: $(NUM_THREADS_LOGS)"
	@output=$$(ls -t $(TARGET_DIR)/release/bench_runner | head -n 1) && \
	$$output -p ./src/bench_files/$(SUBFOLDER) -t $(NUM_THREADS) -l $(NUM_THREADS_LOGS)

run_benches_with_encoded_metrics: start_metrics_container bench_runner
	@echo "Telegraf port: $(TELEGRAF_PORT), Influx port: $(INFLUX_PORT)..."
	@echo "Number of benchmark threads $(NUM_THREADS)"
	@echo "Metrics will be stored at path: ./src/metrics/$(SIZE)"
	@output=$$(ls -t $(TARGET_DIR)/release/bench_runner | head -n 1) && \
	$$output -p ./src/bench_files/$(SUBFOLDER) -t $(NUM_THREADS) \
		metrics -t $(LOCALHOST):$(TELEGRAF_PORT) -i $(LOCALHOST):$(INFLUX_PORT) \
		-o ./src/metrics/$(SUBFOLDER) \
		--influx-auth-token $(INFLUX_AUTH_TOKEN) --influx-org-id $(INFLUX_ORG_ID) -e

run_benches_with_zk_metrics: start_metrics_container bench_runner
	@echo "Telegraf port: $(TELEGRAF_PORT), Influx port: $(INFLUX_PORT)..."
	@echo "Number of benchmark threads $(NUM_THREADS)"
	@echo "Metrics will be stored at path: ./src/metrics/$(SUBFOLDER)"
	@output=$$(ls -t $(TARGET_DIR)/release/bench_runner | head -n 1) && \
	$$output -p ./src/bench_files/$(SUBFOLDER) -t $(NUM_THREADS) \
		metrics -t $(LOCALHOST):$(TELEGRAF_PORT) -i $(LOCALHOST):$(INFLUX_PORT) \
		-o ./src/metrics/$(SUBFOLDER) \
		--influx-auth-token $(INFLUX_AUTH_TOKEN) --influx-org-id $(INFLUX_ORG_ID) \
		measurements zk_vm zk_proving

run_benches_with_gas_constants_and_zk_metrics: start_metrics_container 
	@echo "Running benchmark files path: ./src/bench_files/$(SUBFOLDER)."
	@mkdir -p ./src/metrics/$(SUBFOLDER)
	@cargo build -r --bin bench_runner --features gas-constant-estimation --target-dir $(TARGET_DIR)
	@echo "Telegraf port: $(TELEGRAF_PORT), Influx port: $(INFLUX_PORT)..."
	@echo "Number of benchmark threads $(NUM_THREADS)"
	@echo "Metrics will be stored at path: ./src/metrics/$(SUBFOLDER)"
	@output=$$(ls -t $(TARGET_DIR)/release/bench_runner | head -n 1) && \
	$$output -p ./src/bench_files/$(SUBFOLDER) -t $(NUM_THREADS) \
		metrics -t "tcp://$(LOCALHOST):$(TELEGRAF_PORT)" -i $(LOCALHOST):$(INFLUX_PORT) \
		-o ./src/metrics/$(SUBFOLDER) \
		--influx-auth-token $(INFLUX_AUTH_TOKEN) --influx-org-id $(INFLUX_ORG_ID) \
		measurements zk_vm zk_proving gas_constant_usage

kill_bench_runner:
	@rm -rf ./src/metrics/$(SUBFOLDER)
	@pkill -f 'bench_runner' -9

clean_bench_files:
	@rm -rf ./src/bench_files/$(SUBFOLDER)

run_benches_as_background_task:
	@NO_COLOR=1 nohup make run_benches_with_gas_constants_and_zk_metrics > logs_$(SUBFOLDER).log 2>&1 &

restart_benches: kill_bench_runner restart_metrics_container run_benches_as_background_task

clean_and_restart_all: clean_bench_files generate_benches restart_benches