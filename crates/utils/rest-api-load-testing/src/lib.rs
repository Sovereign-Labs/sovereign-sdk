#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
mod report;
use clap::Parser;
pub use report::*;
mod req_sender;
mod scenario;
pub use req_sender::*;
use scenario::{
    ConcurrentUsersSameConnectionPool, ConcurrentUsersScenarioConfig, ConnConfig, TestScenario,
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Number of users to simulate.
    #[arg(long, default_value_t = 10)]
    nb_of_users: usize,

    /// Number of requests per user.
    #[arg(long, default_value_t = 10)]
    nb_of_requests_per_user: usize,

    /// Connection mode.
    #[arg(value_enum, default_value_t = ConnConfig::SharedConnectionPool)]
    connection_mode: ConnConfig,
}

/// Starts the measurement process.
pub async fn start(requests: Requests) -> Summary {
    let args = Args::parse();

    assert!(
        args.nb_of_users > 0,
        "Number of users must be greater than 0"
    );
    assert!(
        args.nb_of_requests_per_user > 0,
        "Number of requests per user must be greater than 0"
    );

    let config = ConcurrentUsersScenarioConfig {
        nb_of_users: args.nb_of_users,
        nb_of_requests_per_user: args.nb_of_requests_per_user,
        connection_config: args.connection_mode,
    };
    let test_scenario = ConcurrentUsersSameConnectionPool::new(config);
    test_scenario.start_experiment(requests).await
}
