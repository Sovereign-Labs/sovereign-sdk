pub mod client;
pub mod full;

#[derive(Debug, Clone, Copy)]
pub(crate) enum RollupNamespace {
    Batch,
    Proof,
}

impl std::fmt::Display for RollupNamespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RollupNamespace::Batch => {
                write!(f, "batch")
            }
            RollupNamespace::Proof => {
                write!(f, "proof")
            }
        }
    }
}
