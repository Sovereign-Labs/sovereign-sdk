mod concurrent_users;
use std::time::Instant;

pub(crate) use concurrent_users::*;

use crate::{Measurement, RequestSender, Requests, Summary};

/// A test scenario controls the number of concurrent tasks sending messages,
/// the frequency, and other related parameters.
pub trait TestScenario {
    /// Sends requests and creates a report.
    async fn start_experiment(&self, requests: Requests) -> Summary;
}

async fn measurement(request_sender: &RequestSender, url: &str) -> Measurement {
    let started = Instant::now();
    let output = request_sender.request(url).await;

    Measurement {
        time: started.elapsed().as_nanos(),
        output,
    }
}
