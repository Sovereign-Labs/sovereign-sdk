.PHONY: help

PROVER_DIRS := examples/demo-rollup/provers/risc0/guest-mock \
               examples/demo-rollup/provers/risc0/guest-celestia \
  			   examples/demo-rollup/provers/sp1/guest-mock \
			   examples/demo-rollup/provers/sp1/guest-celestia

# Absolutely all dirs
ALL_DIRS := $(PROVER_DIRS)

# We run `cargo hack` with the `--partition 1/1` by default, but overrides allow
# CI to parallelize checks.
CARGO_HACK_PARTITION_N ?= 1
CARGO_HACK_PARTITION_M ?= 1

# Default is 256[^1], but `proptest` can be slow[^2] and local testing is not
# the place to run expensive, long-running tests with property checking. That's
# better left to CI.
#
# Ideally, one day we'd be able to increase this number without affecting
# developer productivity.
#
# [^1]: https://proptest-rs.github.io/proptest/proptest/tutorial/config.html
# [^2]: https://github.com/proptest-rs/proptest/issues/286
export PROPTEST_CASES := 50

help: ## Display this help message
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the project
	@cargo build

clean: ## Cleans compiled
	@cargo clean

check-provers:   ## cargo check in non attached crates
	@set -e; for dir in $(PROVER_DIRS); do \
		echo "$$(date) Running cargo fmt + check in $$dir"; \
		cargo +nightly fmt --all --check --quiet --manifest-path "$$dir/Cargo.toml"; \
		cargo check --all-targets --all-features --manifest-path "$$dir/Cargo.toml"; \
	done

total-clean: clean
total-clean:
	$(MAKE) -C examples/demo-rollup clean;
	$(MAKE) -C examples/demo-rollup clean-wallet;
	@for dir in $(ALL_DIRS); do \
    	echo "Running cargo clean in $$dir"; \
    	(cargo clean --manifest-path "$$dir/Cargo.toml"); \
    done;
	rm -rf "examples/demo-rollup/tests/evm/uniswap/node_modules";
	rm -rf "soak_data/examples/demo-rollup/sov-soak-testing/soak_data"

test:  ## Runs test suite using next test
	@cargo nextest run --no-fail-fast --status-level skip --all-features

test-all: ## Runs test suite using nextest, across the whole workspace
	cargo switcheroo save _backup
	cargo switcheroo disable
	$(MAKE) test
	cargo switcheroo set _backup

test-default-features:  ## Runs test suite using default features
	@cargo nextest run --no-fail-fast --status-level skip

install-dev-tools:  ## Installs all necessary cargo helpers
install-dev-tools: install-risc0-toolchain install-sp1-toolchain
	rustup update nightly
	## Backup VS Code settings to `.vscode/settings.json.bak`.
	cp .vscode/settings.json .vscode/settings.json.bak || true
	## Install the default suggested VS Code settings.
	cp .vscode/settings.default.json .vscode/settings.json
	cargo install cargo-llvm-cov
	cargo install cargo-hack
	cargo install cargo-udeps
	cargo install cargo-deny
	cargo install flaky-finder
	cargo install cargo-insta
	cargo install cargo-nextest --locked
	cargo install zepter
	cargo +stable install cargo-dylint dylint-link
	rustup target add wasm32-unknown-unknown

install-risc0-toolchain:  ## install risc0 toolchain
	curl -L https://risczero.com/install | bash
	~/.risc0/bin/rzup install cargo-risczero 2.0.2
	~/.risc0/bin/rzup install rust 1.88.0
	~/.risc0/bin/rzup install cpp 2024.1.5
	@echo "Risc0 toolchain version:"
	cargo +risc0 --version

install-sp1-toolchain:  ## install SP1 toolchain
	curl -L https://raw.githubusercontent.com/succinctlabs/sp1/main/sp1up/install | bash
	~/.sp1/bin/sp1up $${GITHUB_TOKEN:+--token "$$GITHUB_TOKEN"} --version 5.0.8 --c-toolchain
	~/.sp1/bin/cargo-prove prove --version
	~/.sp1/bin/cargo-prove prove install-toolchain
	@echo "SP1 toolchain version:"
	rustc +succinct --version

lint:  ## cargo fmt, check and clippy.
	## fmt first, because it's the cheapest
	cargo +nightly fmt --all --check
	cargo check --all-targets --all-features
	## Invokes Zepter multiple times because fixes sometimes unveal more underlying issues.
	zepter
	zepter
	zepter
	if which cargo-dylint > /dev/null; then \
		cargo dylint --all; \
	else \
		echo "cargo-dylint not found, skipping dylint check"; \
	fi
	$(MAKE) clippy

lint-all: ## cargo fmt, check and clippy, across the whole workspace
	cargo switcheroo save _backup
	cargo switcheroo disable
	$(MAKE) lint
	cargo switcheroo set _backup

clippy:  ## runs cargo clippy. skips clippy on guest code since it's not supported by risc0
	SKIP_GUEST_BUILD=1 cargo clippy --all-targets --all-features -- -A clippy::too_many_arguments

cargo-deny-check-licenses:
	cargo deny check licenses

cargo-deny-check:   ## Runs a global cargo-deny check, not just the licenses.
	cargo deny check --hide-inclusion-graph

lint-fix:  ## cargo fmt, fix and clippy. Skip clippy on guest code since it's not supported by risc0
	cargo +nightly fmt --all
	cargo fix --allow-dirty
	SKIP_GUEST_BUILD=1 cargo clippy --fix --allow-dirty -- -A clippy::too_many_arguments

check-features: ## Checks that project compiles with all combinations of features.
	cargo hack check --feature-powerset --exclude-features default --partition $(CARGO_HACK_PARTITION_N)/$(CARGO_HACK_PARTITION_M) --all-targets

check-features-default-targets:
	cargo hack check --feature-powerset --exclude-features default --partition $(CARGO_HACK_PARTITION_N)/$(CARGO_HACK_PARTITION_M)

check-constant-overriding-is-disabled-in-release-mode:
	# Passes in release mode...
	SOV_TEST_CONST_OVERRIDE_CHAIN_ID=1 cargo test -p sov-modules-api --profile release-with-opt-level-0 assert_chain_id_was_not_overridden
	# ...but not with standard test profile
	if SOV_TEST_CONST_OVERRIDE_CHAIN_ID=1 cargo test -p sov-modules-api assert_chain_id_was_not_overridden; then \
		echo "Check succeeded, but was expected to fail!"; \
		exit 1; \
	fi
	@echo "Check succeeded!"

find-unused-deps: ## Prints unused dependencies for project. Note: requires nightly
	cargo +nightly udeps --all-targets --all-features

find-flaky-tests:  ## Runs tests over and over to find if there's flaky tests
	flaky-finder -j16 -r320 --continue "cargo test -- --nocapture"

coverage: ## Coverage in lcov format
	SP1_PROVER=mock cargo llvm-cov nextest --locked --all-features --lcov --output-path lcov.info

coverage-html: ## Coverage in HTML format
	SP1_PROVER=mock cargo llvm-cov nextest --locked --all-features --html

dry-run-publish: 
	yq '.[]' packages_to_publish.yml | xargs -I _ cargo publish --allow-dirty --dry-run -p _

docs:  ## Generates documentation locally
	cargo doc --open

docs-generate: ## Generate documentation but don't open it, to verify that it would pass CI.
	cargo doc --no-deps --all-features

doctest:
	cargo test --doc --all-features

mini-ci: ## Runs multiple checks that can most often fail CI as a single command: lint, test, and doctest.
	cargo switcheroo save _backup
	cargo switcheroo disable
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) doctest
	$(MAKE) docs-generate
	$(MAKE) check-provers
	cargo switcheroo set _backup

start-obs: ## Start docker containers needed for observability stack: influxdb, grafana, etc
	./scripts/start_observability.sh

stop-obs: ## Stops observability stack
	./scripts/stop_observability.sh