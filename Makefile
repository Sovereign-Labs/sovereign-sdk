
install-dev-tools:
	cargo install cargo-llvm-cov

check: export RUSTFLAGS := -D warnings
check:
	cargo fmt
	cargo check
	# Disabled until whole code base is checked
	#cargo clippy


coverage:
	cargo llvm-cov --locked --all-features --html