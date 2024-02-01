#!/usr/bin/env bash

# `cargo-publish` requires all dependencies to have a version specified (e.g. it
# can't be sourced from `git` of `path` alone). This script checks that all
# packages we intend to publish have a version specified in their `Cargo.toml` for all
# dependencies.

set -Euo pipefail

yq '.[]' packages_to_publish.yml | while read -r pkg; do
	echo "Checking crate $pkg..."

    output=$(cargo publish --allow-dirty --dry-run -p "$pkg")

	# Check if the output contains the error message we're looking for.
	#
	# `cargo publish` may fail for many other reasons, but it'd be too
	# hard to reason about them all. This is the only one we can expect
	# to catch reliably.
	echo "$output" | grep -q "all dependencies must have a version specified when publishing."
	if [ $? -eq 0 ]; then
	    echo "Error: Found problematic output for crate $pkg."
	    exit 1
	fi
done

echo "All crates processed successfully."
