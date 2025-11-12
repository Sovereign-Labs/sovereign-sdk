use alloy_pubsub::Subscription;
use serde::de::DeserializeOwned;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

pub enum RecvManyError {
    Closed,
}

impl From<RecvError> for RecvManyError {
    fn from(err: RecvError) -> Self {
        match err {
            RecvError::Lagged(n) => {
                panic!("Subscription lagged by {n} during recv");
            }
            RecvError::Closed => Self::Closed,
        }
    }
}

pub trait RecvMany {
    async fn recv_many(&mut self) -> Result<usize, RecvManyError>;
}

impl<T: DeserializeOwned> RecvMany for Subscription<T> {
    async fn recv_many(&mut self) -> Result<usize, RecvManyError> {
        self.recv().await?;
        let mut count = 1;
        loop {
            match self.try_recv() {
                Ok(_) => count += 1,
                Err(TryRecvError::Lagged(n)) => {
                    panic!("Subscription lagged by {n} during try_recv");
                }
                Err(TryRecvError::Closed) => return Err(RecvManyError::Closed),
                Err(TryRecvError::Empty) => break,
            }
        }
        Ok(count)
    }
}
