# JMT

This crate is an implementation of a [jellyfish merkle tree,](https://developers.diem.com/papers/jellyfish-merkle-tree/2021-01-14.pdf)
made generic over the hash function and digest size.
It is based on the [implementation by Aptos-Labs](https://github.com/aptos-labs/aptos-core/tree/6acd52f07650988d28b9f7b13f5d131f3a3ca179),
but has been modified to be used as dependency-minimized standalone package (on top of addition of generics).

## Warning

This code has not been audited and is still under development.
Do not use in a production setting.

## Feature Flags

- "metrics": enable Prometheus metrics. The following counters are enabled.

  - JELLYFISH_LEAF_ENCODED_BYTES: the number of bytes serialized as leaves
  - JELLYFISH_INTERNAL_ENCODED_BYTES: the number of bytes serialized as internal nodes
  - JELLYFISH_LEAF_COUNT: the number of leaves in the tree
  - JELLYFISH_LEAF_DELETION_COUNT: the number of leaves deleted from the tree

- "rayon" uses rayon to parallelize insertions, giving improved performance on multi-core systems

- "fuzzing" enables additional functionality for property-based testing using the proptest library

## License

Licensed under the [Apache License, Version
2.0](./LICENSE).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.
