pub mod blob;
pub mod block;
pub mod config;
pub mod header;

type BoxError = anyhow::Error;

pub mod prelude {
    pub use super::blob::AvailDABlob;
    pub use super::header::AvailHeader;
}
