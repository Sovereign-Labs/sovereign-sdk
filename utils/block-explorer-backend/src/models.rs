use std::fmt::Display;

const MAX_PAGINATION_SIZE: u32 = 100;

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

impl<T> PaginationQuery<T> {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.size > MAX_PAGINATION_SIZE {
            anyhow::bail!(
                "Pagination size cannot be greater than {}",
                MAX_PAGINATION_SIZE
            );
        }

        if self.size == 0 {
            anyhow::bail!("Pagination size cannot be zero");
        }

        if self.after.is_some() && self.before.is_some() {
            anyhow::bail!("Cannot paginate with both `before` and `after`");
        }

        Ok(())
    }
}

fn default_page_size() -> u32 {
    25
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct SortingQuery<T> {
    #[serde(rename = "sort[by]", default)]
    pub by: T,
    #[serde(rename = "sort[direction]", default)]
    pub direction: SortingQueryDirection,
}

impl<T> SortingQuery<T> {
    pub fn map_to_string<F>(&self, f: F) -> SortingQuery<&'static str>
    where
        F: Fn(&T) -> &'static str,
    {
        SortingQuery {
            by: f(&self.by),
            direction: self.direction,
        }
    }
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
    #[serde(rename = "filter[hash]", skip_serializing_if = "Option::is_none")]
    pub hash: Option<HexString>,
    #[serde(rename = "filter[height]", skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(rename = "filter[parentHash]", skip_serializing_if = "Option::is_none")]
    pub parent_hash: Option<HexString>,
    #[serde(flatten)]
    pub pagination: PaginationQuery<HexString>,
    #[serde(flatten)]
    pub sorting: SortingQuery<BlocksQuerySortBy>,
}

#[derive(Debug, Copy, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlocksQuerySortBy {
    Height,
    Timestamp,
}

impl Default for BlocksQuerySortBy {
    fn default() -> Self {
        BlocksQuerySortBy::Height
    }
}

impl BlocksQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.hash.is_some() && self.height.is_some() {
            anyhow::bail!("Cannot filter by both hash and height");
        }
        if self.hash.is_some() && self.parent_hash.is_some() {
            anyhow::bail!("Cannot filter by both hash and parent hash");
        }

        self.pagination.validate()?;

        Ok(())
    }

    fn default_sorting() -> SortingQuery<BlocksQuerySortBy> {
        SortingQuery {
            by: BlocksQuerySortBy::Height,
            direction: SortingQueryDirection::Descending,
        }
    }
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

impl EventsQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.id.is_some() {
            if self.offset.is_some()
                || self.tx_hash.is_some()
                || self.tx_height.is_some()
                || self.key.is_some()
            {
                anyhow::bail!("Cannot filter by both id and other fields");
            }
        }

        // TODO

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum TransactionsQueryFilter {
    Number(u64),
    Hash(HexString),
    Batch(u64, u64),
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionsQuery {
    #[serde(flatten)]
    pub filter: Option<TransactionsQueryFilter>,
    #[serde(flatten)]
    pub pagination: PaginationQuery<HexString>,
    /// Transactions can only ever be sorted by their ID i.e. number.
    #[serde(flatten)]
    pub sorting: SortingQuery<TransactionsQuerySortBy>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionsQuerySortBy {
    Id,
}

impl Default for TransactionsQuerySortBy {
    fn default() -> Self {
        TransactionsQuerySortBy::Id
    }
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
