// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use once_cell::sync::Lazy;
use prometheus::{
    exponential_buckets, register_histogram_vec, register_int_counter_vec, HistogramVec,
    IntCounterVec,
};

pub static SCHEMADB_ITER_LATENCY_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "schemadb_iter_latency_seconds",
        // metric description
        "Schemadb iter latency in seconds",
        // metric labels (dimensions)
        &["cf_name"],
        exponential_buckets(/*start=*/ 1e-6, /*factor=*/ 2.0, /*count=*/ 22).unwrap(),
    )
    .unwrap()
});

pub static SCHEMADB_ITER_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "schemadb_iter_bytes",
        // metric description
        "Schemadb iter size in bytes",
        // metric labels (dimensions)
        &["cf_name"]
    )
    .unwrap()
});

pub static SCHEMADB_GET_LATENCY_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "schemadb_get_latency_seconds",
        // metric description
        "Schemadb get latency in seconds",
        // metric labels (dimensions)
        &["cf_name"],
        exponential_buckets(/*start=*/ 1e-6, /*factor=*/ 2.0, /*count=*/ 22).unwrap(),
    )
    .unwrap()
});

pub static SCHEMADB_GET_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "schemadb_get_bytes",
        // metric description
        "Schemadb get call returned data size in bytes",
        // metric labels (dimensions)
        &["cf_name"]
    )
    .unwrap()
});

pub static SCHEMADB_BATCH_COMMIT_LATENCY_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "schemadb_batch_commit_latency_seconds",
        // metric description
        "Schemadb schema batch commit latency in seconds",
        // metric labels (dimensions)
        &["db_name"],
        exponential_buckets(/*start=*/ 1e-3, /*factor=*/ 2.0, /*count=*/ 20).unwrap(),
    )
    .unwrap()
});

pub static SCHEMADB_BATCH_COMMIT_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "schemadb_batch_commit_bytes",
        // metric description
        "Schemadb schema batch commit size in bytes",
        // metric labels (dimensions)
        &["db_name"]
    )
    .unwrap()
});

pub static SCHEMADB_PUT_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "sov_schema_db_put_bytes",
        // metric description
        "sov_schema_db put call puts data size in bytes",
        // metric labels (dimensions)
        &["cf_name"]
    )
    .unwrap()
});

pub static SCHEMADB_DELETES: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!("storage_deletes", "Storage delete calls", &["cf_name"]).unwrap()
});

pub static SCHEMADB_BATCH_PUT_LATENCY_SECONDS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        // metric name
        "sov_schema_db_batch_put_latency_seconds",
        // metric description
        "sov_schema_db schema batch put latency in seconds",
        // metric labels (dimensions)
        &["db_name"],
        exponential_buckets(/*start=*/ 1e-3, /*factor=*/ 2.0, /*count=*/ 20).unwrap(),
    )
    .unwrap()
});
