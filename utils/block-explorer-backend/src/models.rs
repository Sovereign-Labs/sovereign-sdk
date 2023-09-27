use std::fmt::Display;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Event {
    pub id: i64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct PaginationQuery<T> {
    #[serde(rename = "page[size]", default = "default_page_size")]
    pub size: u32,
    #[serde(
        rename = "page[after]",
        default = "Option::default",
        skip_serializing_if = "Option::is_none"
    )]
    pub after: Option<T>,
    #[serde(
        rename = "page[before]",
        default = "Option::default",
        skip_serializing_if = "Option::is_none"
    )]
    pub before: Option<T>,
}

fn default_page_size() -> u32 {
    25
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct SortingQuery {
    #[serde(rename = "sort[by]")]
    pub by: String,
    #[serde(rename = "sort[direction]")]
    pub direction: SortingQueryDirection,
}

#[derive(Debug, Copy, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SortingQueryDirection {
    Ascending,
    Descending,
}

impl Default for SortingQueryDirection {
    fn default() -> Self {
        SortingQueryDirection::Ascending
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlocksQuery {
    #[serde(rename = "filter[hash]")]
    pub hash: Option<HexString>,
    #[serde(rename = "filter[height]")]
    pub height: Option<i64>,
    #[serde(rename = "filter[parentHash]")]
    pub parent_hash: Option<HexString>,
    #[serde(flatten)]
    pagination: PaginationQuery<HexString>,
    #[serde(flatten)]
    sorting: SortingQuery,
}

#[derive(Debug, serde::Deserialize)]
pub struct EventsQuery {
    /// Cumulative event ID across all transactions.
    pub id: Option<i64>,
    /// Offset within the transaction's events.
    pub offset: Option<i64>,
    pub tx_hash: Option<HexString>,
    pub tx_height: Option<i64>,
    pub key: Option<HexString>,
}

/// A newtype wrapper around [`Vec<u8>`] which is serialized as a
/// 0x-prefixed hex string.
#[derive(Debug, Clone)]
pub struct HexString(pub Vec<u8>);

impl AsRef<Vec<u8>> for HexString {
    fn as_ref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl Display for HexString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl serde::Serialize for HexString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        hex::encode(&self.0).serialize(serializer)
    }
}

impl<'a> serde::Deserialize<'a> for HexString {
    fn deserialize<D>(deserializer: D) -> Result<HexString, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let string = String::deserialize(deserializer)?;
        // We ignore the 0x prefix if it exists.
        let s = string.strip_prefix("0x").unwrap_or(&string);

        hex::decode(s)
            .map_err(|e| anyhow::anyhow!("Failed to decode hex: {}", e))
            .map(HexString)
            .map_err(serde::de::Error::custom)
    }
}
