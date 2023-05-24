install-dev-tools:
	cargo install cargo-llvm-cov
	cargo install cargo-hack
	cargo install cargo-udeps
	cargo install flaky-finder

fix-checks: export RUSTFLAGS := -D warnings
fix-checks:
	cargo fmt --all
	cargo fix --allow-dirty


lint:
	cargo check
	cargo clippy --all

check-features:
	cargo hack --feature-powerset check

# Note: requires nightly
unused-dependencies:
	cargo udeps --all-targets

flaky-finder:
	flaky-finder -j16 -r320 --continue "cargo test -- --nocapture"

coverage:
	cargo llvm-cov --locked --all-features --lcov --output-path lcov.info

coverage-html:
	cargo llvm-cov --locked --all-features --html
