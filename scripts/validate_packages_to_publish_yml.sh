#!/usr/bin/env bash

set -Euo pipefail

# This script makes sure that all workspace crates that are marked as releasable
# are also listed in packages_to_publish.yml. In addition, it makes sure that
# all packages listed in packages_to_publish.yml have all basic crate metadata and don't
# generate warnings when packaged.
#
# Important: this script is assumed to be cheap to run by our GH Actions
# workflows. Create another script if you need to add checks that are more expensive.

# Somewhat unintuitively, `cargo metadata` returns `"publish": null` for publishable packages.
# From the docs at https://doc.rust-lang.org/cargo/commands/cargo-metadata.html#output-format:
#    /* List of registries to which this package may be published.
#       Publishing is unrestricted if null, and forbidden if an empty array. */
releasable_packages=$(cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | select(.publish == null) | .name')

echo "Releasable packages according to cargo-metadata:"
echo "$releasable_packages" | sed 's/^/- /'
echo ""

echo "Releasable packages according to packages_to_publish.yml:"
yq '.[]' packages_to_publish.yml | sed 's/^/- /'
echo ""

echo "Validating packages_to_publish.yml..."

status=0
while read -r pkg; do
    if yq -e "[\"$pkg\"] - . | length == 1" packages_to_publish.yml > /dev/null 2>&1; then
        printf "%40s | ERR is releasable but NOT found in packages_to_publish.yml\n" "$pkg"
		status=1
	fi
done <<< "$releasable_packages"

echo ""
echo "Validating the present of package metadata for all packages_to_publish.yml entries..."

while read -r pkg; do
	# Capture both stdour and stderr.
	output=$(cargo package --allow-dirty -p $pkg --list 2>&1)
    if echo "$output" | grep -q "warning:"; then
        printf "%40s | ERR warnings found:\n" "$pkg"
		echo "$output" | grep "warning:"
		status=1
	fi
done < <(yq '.[]' packages_to_publish.yml)

echo ""
if [ $status -eq 1 ]; then
	echo "Validation failed."
	exit 1
fi

echo "Validation successful, everything okay."
echo "Goodbye!"
