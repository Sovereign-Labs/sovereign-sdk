pub type DefaultHash = Sha2Hash;

/// The output of a sha2-256 hash
///
#[derive(Debug, Default, Clone)]
pub struct Sha2Hash([u8; 32]);
