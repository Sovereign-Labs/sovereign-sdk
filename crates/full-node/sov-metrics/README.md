# Description

Backend agnostic metrics tracking for Sovereign Rollups.

This crate provides a flexible and backend-agnostic framework for tracking custom metrics in Sovereign Rollups.
The primary interface allows developers to define and record metrics that are serialized in
the [Telegraf line protocol](https://docs.influxdata.com/influxdb/cloud/reference/syntax/line-protocol/).
Metrics are timestamped automatically and can only be tracked in **native mode**.

## Architecture overview

### **Tracker**

The Tracker is responsible for recording metrics and associating them with a timestamp.
It collects the data and forwards it to the Publisher for processing.

### **Publisher**

The Publisher buffers incoming metrics and efficiently sends them to Telegraf.
For optimal performance, metrics are serialized and published in a background thread,
ensuring minimal impact on the main application thread.

For a more detailed overview of the entire observability stack and its integration with Grafana, refer to this tutorial:
[Tutorial about observability](https://sovlabs.notion.site/Tutorial-Getting-started-with-Grafana-Cloud-17e47ef6566b80839fe5c563f5869017?pvs=74)

## Defining Custom Metrics

To define a custom metric, follow these steps:

1. Create a struct representing your metric. The struct can include any number
   of [fields and tags](https://docs.influxdata.com/influxdb/v1/concepts/key_concepts/).
2. Implement the [`Metric`] trait for your struct. This implementation should serialize metrics using the
   Telegraf line protocol format.
   > **Note**: Be mindful of special characters in metric names, field names, and tag names. You do **not** need to
   > explicitly record the metric's timestamp—this is handled by the `sov_metrics` crate.

### Example

Below is an example of defining a custom metric:

```rust
use std::io::Write;

#[derive(Debug)]
pub struct MyCustomMetric {
    time: std::time::Duration,
    value: u64,
    tag: u64,
}

impl sov_metrics::Metric for MyCustomMetric {
    fn measurement_name(&self) -> &'static str {
        "my_custom_metric"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} my_tag={} my_value={},my_time_spent_ms={}",
            self.measurement_name(),
            self.tag,
            self.value,
            self.time.as_millis(),
        )
    }
}
```

In this example:

- `MyCustomMetric` tracks a time duration (`time`), a numerical value (`value`), and a tag (`tag`).
- The `serialize_for_telegraf` method ensures the metric is properly formatted for tracking.

## Tracking Metrics

To track metrics, use the [`track_metrics`] function and pass a closure that contains your metrics logic.
Metrics can only be tracked when the code is compiled with the `native` feature flag enabled.

### Example

Here's an example of tracking metrics during the execution of some computational logic:

```rust
# #[derive(Debug)]
# struct MyCustomMetric {
#     time: std::time::Duration,
#     value: u64,
#     tag: u64,
# }
#
# impl sov_metrics::Metric for MyCustomMetric {
#     fn measurement_name(&self) -> &'static str {
#         "a"
#     }
#
#     fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
#        use std::io::Write;
#         write!(
#             buffer,
#             "{} tag={} v={},time={}",
#             self.measurement_name(),
#             self.tag,
#             self.value,
#             self.time.as_millis(),
#         )
#   }
# }

fn my_code(input: u64) -> u64 {
    sov_metrics::start_timer!(start_operation);
    let result: u64 = some_expensive_operation(input);
    sov_metrics::save_elapsed!(my_operation_time SINCE start_operation);
    #[cfg(feature = "native")]
    {
        sov_metrics::track_metrics(|tracker| {
            // Timestamp will be added at this moment.
            tracker.submit(
                MyCustomMetric { value: result, tag: input, time: my_operation_time }
            );
            // More metrics can be tracked at once
        })
    }
    result
}


fn some_expensive_operation(input: u64) -> u64 {
   input * 2
}
```

### Key Points:

- The `track_metrics` function records all metrics during the function's execution.
- Timestamping is handled automatically when metrics are being tracked.
