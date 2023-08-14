.PHONY: help

help: ## Display this help message
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the the project
	@cargo build

clean: ## Cleans compiled
	@cargo clean

test-legacy: ## Runs test suite with output from tests printed
	@cargo test -- --nocapture -Zunstable-options --report-time

test:  ## Runs test suite using next test
	@cargo nextest run --workspace --all-features

install-dev-tools:  ## Installs all necessary cargo helpers
	cargo install cargo-llvm-cov
	cargo install cargo-hack
	cargo install cargo-udeps
	cargo install flaky-finder
	cargo install cargo-nextest --locked

lint:  ## cargo check and clippy
	## fmt first, because it's the cheapest
	cargo fmt --all --check
	cargo check --all-targets --all-features
	cargo clippy --all-targets --all-features

lint-fix:  ## cargo fmt, fix and clippy
	cargo fmt --all
	cargo fix --allow-dirty
	cargo clippy --fix --allow-dirty

check-features: ## Checks that project compiles with all combinations of features
	cargo hack --feature-powerset check

find-unused-deps: ## Prints unused dependencies for project. Note: requires nightly
	cargo udeps --all-targets --all-features

find-flaky-tests:  ## Runs tests over and over to find if there's flaky tests
	flaky-finder -j16 -r320 --continue "cargo test -- --nocapture"

coverage: ## Coverage in lcov format
	cargo llvm-cov --locked --lcov --output-path lcov.info

coverage-html: ## Coverage in HTML format
	cargo llvm-cov --locked --all-features --html

dry-run-publish: 
	cat packages_to_publish.txt | xargs -I _ cargo publish --allow-dirty --dry-run -p _

docs:  ## Generates documentation locally
	cargo doc --open
