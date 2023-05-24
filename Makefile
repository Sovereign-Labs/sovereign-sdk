
install-dev-tools:
	cargo install cargo-llvm-cov
	cargo install cargo-hack
	cargo install cargo-udeps


fix-checks: export RUSTFLAGS := -D warnings
fix-checks:
	cargo fmt --all
	cargo fix --allow-dirty

lint:
lint: export RUSTFLAGS := -D warnings:
	cargo check
	cargo clippy --all

check-features:
	cargo hack --feature-powerset check

unused-dependencies:
	cargo +nightly udeps --all-targets

flaky-finder:
	flaky-finder -j16 -r320 --continue "cargo test -- --nocapture"

coverage:
	cargo llvm-cov --locked --all-features --lcov --output-path lcov.info

coverage-html:
	cargo llvm-cov --locked --all-features --html