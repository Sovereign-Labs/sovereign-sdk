.PHONY: help

help: ## Display this help message
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the the project
	@cargo build

clean: ## Cleans compiled
	@cargo clean

test: ## Runs test suite
	cargo test

install-dev-tools:  ## Installs all necessary cargo helpers
	cargo install cargo-llvm-cov
	cargo install cargo-hack
	cargo install cargo-udeps
	cargo install flaky-finder

fix: export RUSTFLAGS := -D warnings
fix:  ## cargo fmt and fix
	cargo fmt --all
	cargo fix --allow-dirty


lint:  ## cargo check and clippy
	cargo check
	cargo clippy --all

check-features: ## Checks that project compiles with all combinations of features
	cargo hack --feature-powerset check

find-unused-deps: ## Prints unused dependencies for project. Note: requires nightly
	cargo udeps --all-targets

find-flaky-tests:  ## Runs tests over and over to find if there's flaky tests
	flaky-finder -j16 -r320 --continue "cargo test -- --nocapture"

coverage: ## Coverage in lcov format
	cargo llvm-cov --locked --all-features --lcov --output-path lcov.info

coverage-html: ## Coverage in HTML format
	cargo llvm-cov --locked --all-features --html

docs:  ## Generates documentation locally
	cargo doc --open
