use avail_subxt::primitives::AppUncheckedExtrinsic;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Confidence {
    pub block: u32,
    pub confidence: f64,
    pub serialised_confidence: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExtrinsicsData {
    pub block: u32,
    pub extrinsics: Vec<AppUncheckedExtrinsic>,
}
