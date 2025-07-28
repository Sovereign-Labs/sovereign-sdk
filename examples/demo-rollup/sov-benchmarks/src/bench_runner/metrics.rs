//! This module can query metrics from telegraph and store them.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Duration;

use futures::{pin_mut, StreamExt};
use reqwest::Client;
use sov_metrics::timestamp;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use tokio::time::interval;
use tracing::{info, trace};

use super::ParsedMetricsParameters;

/// Should be greater or equal to influx's flush interval.
const METRICS_FLUSH_INTERVAL: Duration = Duration::from_secs(200);

async fn get_metrics(
    bench_name: String,
    start_timestamp: u128,
    end_timestamp: u128,
    metrics_params: &ParsedMetricsParameters,
    out_file: &mut File,
) -> anyhow::Result<()> {
    let metrics_query = format!(
        "from(bucket: \"sov-rollup\")
    |> range(start: time(v: {start_timestamp}), stop: time(v: {end_timestamp}))
    |> filter(fn: (r) => r.bench_file == \"{bench_name}\")
    {}",
        metrics_params.query_filter
    );

    let post_addr = format!(
        "http://{}/api/v2/query?org={}",
        metrics_params.influx_address, metrics_params.influx_org_id
    );

    let client = Client::new();

    let mut response_builder = client
        .post(post_addr)
        .bearer_auth(metrics_params.influx_auth_token.clone())
        .header("Accept", "application/csv")
        .header("Content-type", "application/vnd.flux")
        .body(metrics_query);

    // If the metrics are encoded, then we need to add the Accept-Encoding header.
    if metrics_params.encoded {
        response_builder = response_builder.header("Accept-Encoding", "gzip");
    }

    let response = response_builder.send().await?.error_for_status()?;

    // Stream the response to the output file.
    let response_stream = response.bytes_stream();
    let mut writer = BufWriter::new(out_file);

    pin_mut!(response_stream);
    while let Some(chunk) = response_stream.next().await {
        match chunk {
            Ok(chunk) => {
                writer.write_all(&chunk)?;
            }
            Err(e) => {
                // This should never happen.
                // If it does, it means that the response is malformed.
                // We should panic, because we can't recover from this.
                // The response is malformed, so we can't even try to recover.
                panic!("Failed to read response chunk: {e:?}");
            }
        }
    }

    writer.flush()?;

    info!(
        bench = bench_name,
        start = start_timestamp,
        end = end_timestamp,
        output = metrics_params.output_file,
        thread = "metrics_storage",
        "Successfully queried metrics from InfluxDB."
    );

    Ok(())
}

/// Main function that queries metrics from telegraph and stores them to the supplied file.
pub async fn start_metrics_thread(
    bench_name: String,
    initial_timestamp: u128,
    metrics_params: ParsedMetricsParameters,
    shutdown_receiver: tokio::sync::watch::Receiver<()>,
) -> anyhow::Result<()> {
    // Perform health checks for influxdb instance.
    match reqwest::Client::new()
        .get(format!("http://{}/health", metrics_params.influx_address))
        .send()
        .await
    {
        Ok(response) => {
            if response.status() != 200 {
                panic!("Unhealthy influxdb instance: {}. Please ensure that the instance is properly set up.", metrics_params.influx_address);
            }
        }
        Err(_) => {
            panic!("Invalid influxdb address: {}. Please ensure that the address is correct and that the service is running.", metrics_params.influx_address);
        }
    };

    // Query metrics from telegraph and store them to the output file.
    trace!(
        bench = bench_name,
        thread = "metrics_storage",
        "Querying metrics from InfluxDB..."
    );

    let mut output_file = File::create(metrics_params.output_file.clone())
        .expect("Failed to create metrics output file");

    let mut curr_start_timestamp = initial_timestamp;

    // We are storing the previous interval when the current one is finished to ensure all metrics are flushed.
    let mut prev_start_stamp = initial_timestamp;

    let mut interval = interval(METRICS_FLUSH_INTERVAL);

    loop {
        match future_or_shutdown(interval.tick(), &shutdown_receiver).await {
            FutureOrShutdownOutput::Output(_) => {
                trace!(
                    thread = "metrics_storage",
                    output = metrics_params.output_file,
                    "Storing metrics to file..."
                );

                let curr_stamp = timestamp();

                // We store metrics with a delay to ensure that everything is flushed.
                if curr_start_timestamp > prev_start_stamp {
                    get_metrics(
                        bench_name.clone(),
                        prev_start_stamp,
                        curr_start_timestamp,
                        &metrics_params,
                        &mut output_file,
                    )
                    .await?;
                }

                prev_start_stamp = curr_start_timestamp;
                curr_start_timestamp = curr_stamp;
            }
            FutureOrShutdownOutput::Shutdown => {
                // We store the last metrics before exiting.
                info!(
                    bench = bench_name,
                    thread = "metrics_storage",
                    "Metrics storage thread has received a shutdown signal. Exiting..."
                );

                let curr_stamp = timestamp();
                get_metrics(
                    bench_name.clone(),
                    prev_start_stamp,
                    curr_stamp,
                    &metrics_params,
                    &mut output_file,
                )
                .await?;

                break;
            }
        }
    }

    trace!(
        bench = bench_name,
        thread = "metrics_storage",
        "Exited metrics storage thread."
    );

    Ok(())
}
