use clap::ValueEnum;
use tokio::task::JoinSet;

use super::{measurement, TestScenario};
use crate::{Report, RequestSenderFactory, Requests, Summary};

/// Connection configuration for the concurrent users scenario.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum ConnConfig {
    /// All the users share the same connection pool.
    SharedConnectionPool,
    /// Each user has its own connection pool.
    PerUserConnectionPool,
}

pub(crate) struct ConcurrentUsersScenarioConfig {
    pub(crate) nb_of_users: usize,
    pub(crate) nb_of_requests_per_user: usize,
    pub(crate) connection_config: ConnConfig,
}

/// Test scenario where a number of concurrent users send requests to the full node.
/// All the users share the same connection pool.
pub(crate) struct ConcurrentUsersSameConnectionPool {
    config: ConcurrentUsersScenarioConfig,
    request_sender: RequestSenderFactory,
}

impl ConcurrentUsersSameConnectionPool {
    pub fn new(config: ConcurrentUsersScenarioConfig) -> Self {
        let request_sender = match config.connection_config {
            ConnConfig::SharedConnectionPool => RequestSenderFactory::new_shared_conn_pool(),
            ConnConfig::PerUserConnectionPool => {
                RequestSenderFactory::new_individual_conn_pool(config.nb_of_users)
            }
        };

        Self {
            request_sender,
            config,
        }
    }
}

impl TestScenario for ConcurrentUsersSameConnectionPool {
    async fn start_experiment(&self, requests: Requests) -> Summary {
        let nb_of_urls = requests.urls.len();
        let nb_of_users = self.config.nb_of_users;
        let nb_of_requests_per_user = self.config.nb_of_requests_per_user;

        let mut data = Vec::with_capacity(nb_of_urls);

        //For each URL, spawn a user concurrently. Each user sends requests in a busy loop and collects measurements.
        for url in requests.urls {
            let mut set = JoinSet::new();
            for i in 0..nb_of_users {
                let request_sender = self.request_sender.get_req_sender(i);

                let url = url.clone();
                // Spawn a concurrent task for each user.
                set.spawn(async move {
                    let mut measurements_for_user = Vec::with_capacity(nb_of_requests_per_user);
                    // Each user sends requests in a busy loop and collects measurements.
                    for _ in 0..nb_of_requests_per_user {
                        let measurement = measurement(&request_sender, &url).await;
                        measurements_for_user.push(measurement);
                    }
                    measurements_for_user
                });
            }

            // Wait for all the measurements.
            let mut all_measurements = Vec::with_capacity(nb_of_users * nb_of_requests_per_user);
            while let Some(measurements_for_user) = set.join_next().await {
                // Panic if any of the task panicked.
                all_measurements.append(&mut measurements_for_user.unwrap());
            }

            data.push(Report::create_report(url.clone(), all_measurements));
        }

        Summary { data }
    }
}
