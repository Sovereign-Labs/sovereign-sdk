
check: export RUSTFLAGS := -D warnings
check:
	cargo fmt
	cargo check
	# Disabled until whole code base is checked
	#cargo clippy