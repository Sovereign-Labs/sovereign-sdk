// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0
// Adapted from aptos-core/storage/jmt.
// Counters have been relabeled

use once_cell::sync::Lazy;
use prometheus::{register_int_counter, register_int_gauge, IntCounter, IntGauge};

pub static JELLYFISH_LEAF_ENCODED_BYTES: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "jellyfish_leaf_encoded_bytes",
        "jellyfish leaf encoded bytes in total"
    )
    .unwrap()
});

pub static JELLYFISH_INTERNAL_ENCODED_BYTES: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "jellyfish_internal_encoded_bytes",
        "jellyfish total internal nodes encoded in bytes"
    )
    .unwrap()
});

pub static JELLYFISH_LEAF_COUNT: Lazy<IntGauge> = Lazy::new(|| {
    register_int_gauge!(
        "jellyfish_leaf_count",
        "Total number of leaves in the latest JMT"
    )
    .unwrap()
});

pub static JELLYFISH_LEAF_DELETION_COUNT: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "jellyfish_leaf_deletion_count",
        "Number of deletions from the JMT."
    )
    .unwrap()
});
