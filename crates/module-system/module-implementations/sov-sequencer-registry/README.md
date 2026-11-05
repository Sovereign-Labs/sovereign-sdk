# `sov-sequencer-registry` module

The `sov-sequencer-registry` module manages sequencer registration, stake, slashing, and DA address rotation.

## Activating DA address rotation

Fresh chains use `SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT = 0` and an empty `SEQUENCER_DA_ROTATION_UPGRADE_ADDRESSES`. For an existing chain, choose the first execution height `H` with rotation enabled. Stop the node at execution height `H-1` and retain a copy of that finalized database snapshot. Resolve any rollup owner with multiple DA registrations before taking this snapshot; the exporter rejects duplicate owners and includes registrations awaiting withdrawal. Do not resume the old binary between export and deployment: further registrations or withdrawals would invalidate the list.

Export the complete list using the snapshot's rollup configuration. Replace the absolute paths below with your checkout, config, and constants paths. The tool opens the databases with pruning disabled and performs no state migration commits.

```bash
SKIP_GUEST_BUILD=1 SP1_SKIP_PROGRAM_BUILD=1 cargo run --manifest-path /path/to/sovereign-sdk/examples/demo-rollup/Cargo.toml --bin sequencer-registry-upgrade --features sequencer-registry-upgrade -- --da-layer mock --rollup-config-path /path/to/snapshot-rollup.toml --activation-height 100
```

For Celestia, add `--no-default-features --features celestia_da,sequencer-registry-upgrade` in place of `--features sequencer-registry-upgrade`, and use `--da-layer celestia`. The output identifies the execution height, DA slot, source state root, and both TOML constants. The address constant is one whitespace-separated string of canonical DA addresses; export sorts it lexically.

Copy both constants into the chain's constants manifest and rebuild the node and verifier with that same manifest. Verify the compiled values against the same stopped snapshot before deployment:

```bash
CONSTANTS_MANIFEST=/path/to/constants-directory SKIP_GUEST_BUILD=1 SP1_SKIP_PROGRAM_BUILD=1 cargo run --manifest-path /path/to/sovereign-sdk/examples/demo-rollup/Cargo.toml --bin sequencer-registry-upgrade --features sequencer-registry-upgrade -- --da-layer mock --rollup-config-path /path/to/snapshot-rollup.toml --verify-config
```

Verification rejects the wrong execution height, inconsistent storage/ledger roots, duplicate owners, and missing, extra, duplicate, malformed, or noncanonical DA addresses. Activation initializes the reverse index through consensus execution at `H`; the snapshot export does not rewrite deployed state and does not require a `STATE_VERSION` bump. Kernel blob selection can slash a listed registration before the activation hook runs; the hook skips those removed entries. Retain the immutable activation height and complete address list for historical replay on every node and verifier. Configure the chain-hash override for the historical transaction schema separately.
