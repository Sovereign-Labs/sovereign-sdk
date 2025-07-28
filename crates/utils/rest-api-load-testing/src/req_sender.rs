#![allow(dead_code)]
use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, ClientBuilder};

const REQUEST_TIMEOUT: Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone)]
/// The output of a single HTTP request.
pub struct ResponseOutput {
    pub(crate) status: u16,
    pub(crate) body_size: usize,
}

/// The collection of urls to be analyzed
pub struct Requests {
    pub(crate) urls: Vec<Arc<String>>,
}

impl Requests {
    /// Creates `Requests` from a list of endpoints and a host.
    pub fn new(host: &str, endpoints: Vec<String>) -> Self {
        Self {
            urls: endpoints
                .into_iter()
                .map(|e| Arc::new(format!("{host}/{e}")))
                .collect(),
        }
    }
}

/// A request sender that can be used to send requests to the full node.
#[derive(Clone)]
pub(crate) struct RequestSender {
    client: Client,
}

impl RequestSender {
    pub(crate) async fn request(&self, url: &str) -> anyhow::Result<ResponseOutput> {
        let response = self.client.get(url).send().await?;
        let status = response.status().as_u16();
        let body = response.bytes().await?;

        Ok(ResponseOutput {
            status,
            body_size: body.len(),
        })
    }
}

enum ConnMode {
    SharedConnPool(Client),
    IndividualConnPool(Vec<Client>),
}

/// A factory that creates request senders.
#[derive(Clone)]
pub(crate) struct RequestSenderFactory {
    mode: Arc<ConnMode>,
}

impl RequestSenderFactory {
    pub(crate) fn new_shared_conn_pool() -> Self {
        // All the users share the same http client.
        let client = ClientBuilder::new()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap();

        Self {
            mode: Arc::new(ConnMode::SharedConnPool(client)),
        }
    }

    pub(crate) fn new_individual_conn_pool(nb_of_users: usize) -> Self {
        // Each user has its own http client.
        let clients = (0..nb_of_users)
            .map(|_| {
                ClientBuilder::new()
                    .timeout(REQUEST_TIMEOUT)
                    .build()
                    .unwrap()
            })
            .collect();

        Self {
            mode: Arc::new(ConnMode::IndividualConnPool(clients)),
        }
    }

    pub(crate) fn get_req_sender(&self, i: usize) -> RequestSender {
        let client = match self.mode.as_ref() {
            ConnMode::SharedConnPool(client) => client.clone(),
            ConnMode::IndividualConnPool(clients) => clients[i].clone(),
        };
        RequestSender { client }
    }
}
