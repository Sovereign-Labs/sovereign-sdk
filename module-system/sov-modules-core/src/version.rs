#[cfg(feature = "std")]
pub use jmt::Version;

#[cfg(not(feature = "std"))]
pub type Version = u64;
