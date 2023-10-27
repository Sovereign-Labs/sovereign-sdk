.PHONY: help

BINARIES := $(shell awk -F'=[[:space:]]*"|"[[:space:]]*$$' '/^\[\[bin\]\]/,/^$$/{ if ($$1 ~ /name[[:space:]]*/) { print $$2 } }' Cargo.toml)
PROFILE ?= debug
ARGS ?=

help: ## Display this help message
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

check: ## Checks that fuzz member compiles
	cargo check

# Needs nightly for `-Z sanitizer=address`
build-target: ## Build the target fuzz
	cargo +nightly rustc --bin $(TARGET) $(ARGS) -- \
		-C debuginfo=full \
		-C debug-assertions \
		-C passes='sancov-module' \
		-C llvm-args='-sanitizer-coverage-level=3' \
		-C llvm-args='-sanitizer-coverage-inline-8bit-counters' \
		-Z sanitizer=address

build: ## Build the fuzz targets
	@for t in $(BINARIES); do \
		$(MAKE) build-target TARGET=$$t; \
    done

targets: ## Prints all fuzz targets
	@for t in $(BINARIES); do \
		echo $$t; \
	done

run: build-target ## Run the fuzz target
	./target/$(PROFILE)/$(TARGET) -artifact_prefix=artifacts/
