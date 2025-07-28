use std::ops::Div;
use std::sync::Arc;

use anyhow::{bail, Context};

use crate::ResponseOutput;

/// Contains details about a single HTTP request measurement.
#[derive(Debug)]
pub struct Measurement {
    /// The time it took to complete the request in nanoseconds.
    pub time: u128,
    /// Information about the response.
    pub output: anyhow::Result<ResponseOutput>,
}

/// Contains statistics for a single endpoint.
#[derive(Debug, Clone)]
pub struct Report {
    /// The URL of the request.
    pub url: Arc<String>,
    /// The average time it took to complete the request in nanoseconds.
    pub average_time: u128,
    /// The minimum time it took to complete the request in nanoseconds.
    pub min_time: u128,
    /// The maximum time it took to complete the request in nanoseconds.
    pub max_time: u128,
    /// The 95% of the requests finished in less than this time.
    pub p95_time: u128,
}

impl Report {
    pub(crate) fn create_report(
        url: Arc<String>,
        mut measurements: Vec<Measurement>,
    ) -> anyhow::Result<Report> {
        let mut total_time: u128 = 0;
        let len = measurements.len();

        measurements.sort_by(|a, b| a.time.cmp(&b.time));
        let index_p95: usize = measurements.len() * 95 / 100;
        let p95_time = measurements[index_p95].time;
        let min_time = measurements[0].time;
        let max_time = measurements[len - 1].time;

        for measurement in measurements {
            total_time += measurement.time;

            let out = measurement
                .output
                .with_context(|| format!("Querying URL: {url:?} failed."))?;

            if out.status != reqwest::StatusCode::OK {
                bail!("Measurement failed with status: {out:?}, URL: {:?}", url);
            }
        }

        // It is ok to div here, because we are not interested in fractions of a nanosecond.
        let average_time = total_time.div(len as u128);

        Ok(Report {
            url,
            average_time,
            min_time,
            max_time,
            p95_time,
        })
    }
}

/// The summary for all the URLs.
pub struct Summary {
    pub(crate) data: Vec<anyhow::Result<Report>>,
}

impl Summary {
    fn filter_out_errors(self) -> (Vec<Report>, Vec<anyhow::Error>) {
        let mut ok_data = Vec::new();
        let mut not_ok_data = Vec::new();
        for res in self.data {
            match res {
                Ok(data) => ok_data.push(data),
                Err(e) => not_ok_data.push(e),
            }
        }

        (ok_data, not_ok_data)
    }

    fn sort_by_avg_time(mut ok_data: Vec<Report>) -> Vec<Report> {
        ok_data.sort_by(|a, b| a.average_time.cmp(&b.average_time));
        ok_data
    }

    /// Print the summary of the experiment.
    pub fn print_summary(self) {
        let (ok_data, not_ok_data) = self.filter_out_errors();

        if !not_ok_data.is_empty() {
            println!("Some of the measurements failed for the following reasons:");
        }

        for not_ok in not_ok_data {
            println!("{not_ok:?}");
            println!();
        }

        println!();
        println!("Measurement reports sorted by average time taken: ");
        let sorted = Self::sort_by_avg_time(ok_data);
        for report in sorted {
            println!("{report:?}");
        }
    }
}
