use std::str::FromStr;

use super::extractors::QueryValidation;
use super::{Pagination, Sorting, SortingOrder};
use crate::utils::{CommaSeparatedStrings, HexString};

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Event {
    pub id: i64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub enum BlocksQueryFilter {
    #[serde(rename = "filter[number]")]
    Number(u64),
    #[serde(rename = "filter[hash]")]
    Hash(HexString),
    #[serde(rename = "filter[parentHash]")]
    ParentHash(HexString),
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub struct BlocksQuery {
    #[serde(
        flatten,
        default = "Option::default",
        skip_serializing_if = "Option::is_none"
    )]
    pub filter: Option<BlocksQueryFilter>,
    #[serde(flatten)]
    pub pagination: Pagination<i64>,
    #[serde(default = "BlocksQuery::default_sorting")]
    pub sort: Sorting<BlocksQuerySortBy>,
    #[serde(default, skip_serializing_if = "CommaSeparatedStrings::is_empty")]
    pub include: CommaSeparatedStrings,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum BlocksQuerySortBy {
    Number,
    Timestamp,
}

impl FromStr for BlocksQuerySortBy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "number" => Ok(Self::Number),
            "timestamp" => Ok(Self::Timestamp),
            _ => anyhow::bail!("Invalid sort field, only 'number' and 'timestamp' are supported"),
        }
    }
}

impl QueryValidation for BlocksQuery {
    fn validate(&self) -> anyhow::Result<()> {
        match (&self.sort.by, &self.filter) {
            (_, None) => (),
            (BlocksQuerySortBy::Number, Some(BlocksQueryFilter::Number(_))) => (),
            _ => anyhow::bail!("You can only filter and sort by the same field"),
        }

        self.pagination.validate()?;
        self.sort.validate()?;

        Ok(())
    }
}

impl BlocksQuery {
    fn default_sorting() -> Sorting<BlocksQuerySortBy> {
        Sorting {
            by: BlocksQuerySortBy::Number,
            order: SortingOrder::Descending,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub struct BatchesQuery {
    #[serde(flatten)]
    pub pagination: Pagination<i64>,
    #[serde(default = "BatchesQuery::default_sorting")]
    pub sort: Sorting<BatchesQueryBy>,
}

impl QueryValidation for BatchesQuery {
    fn validate(&self) -> anyhow::Result<()> {
        self.pagination.validate()?;
        self.sort.validate()?;

        Ok(())
    }
}

impl BatchesQuery {
    fn default_sorting() -> Sorting<BatchesQueryBy> {
        Sorting {
            by: BatchesQueryBy::Id,
            order: SortingOrder::Descending,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub enum BatchesQueryBy {
    Id,
}

impl FromStr for BatchesQueryBy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "id" => Ok(Self::Id),
            _ => anyhow::bail!("Invalid sort field, only 'id' is supported"),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub struct EventsQuery {
    /// Cumulative event ID across all transactions.
    pub id: Option<i64>,
    /// Offset within the transaction's events.
    pub offset: Option<i64>,
    pub tx_hash: Option<HexString>,
    pub tx_height: Option<i64>,
    pub key: Option<HexString>,
    #[serde(flatten)]
    pub pagination: Pagination<i64>,
    #[serde(default = "EventsQuery::default_sorting")]
    pub sort: Sorting<EventsQueryBy>,
    #[serde(default, skip_serializing_if = "CommaSeparatedStrings::is_empty")]
    pub include: CommaSeparatedStrings,
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum EventsQueryBy {
    Id,
}

impl FromStr for EventsQueryBy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "id" => Ok(Self::Id),
            _ => anyhow::bail!("Invalid sort field, only 'id' is supported"),
        }
    }
}

impl EventsQuery {
    fn default_sorting() -> Sorting<EventsQueryBy> {
        Sorting {
            by: EventsQueryBy::Id,
            order: SortingOrder::Ascending,
        }
    }
}

impl QueryValidation for EventsQuery {
    fn validate(&self) -> anyhow::Result<()> {
        if self.id.is_some()
            && (self.offset.is_some()
                || self.tx_hash.is_some()
                || self.tx_height.is_some()
                || self.key.is_some())
        {
            anyhow::bail!("Cannot filter by both id and other fields");
        }

        self.pagination.validate()?;
        self.sort.validate()?;

        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(untagged)]
pub enum TransactionsQueryFilter {
    Number(u64),
    Hash(HexString),
    Batch(u64, u64),
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub struct TransactionsQuery {
    #[serde(
        flatten,
        default = "Option::default",
        skip_serializing_if = "Option::is_none"
    )]
    pub filter: Option<TransactionsQueryFilter>,
    #[serde(flatten)]
    pub pagination: Pagination<i64>,
    /// Transactions can only ever be sorted by their ID i.e. number.
    #[serde(default = "TransactionsQuery::default_sorting")]
    pub sort: Sorting<TransactionsQuerySortBy>,
    #[serde(default, skip_serializing_if = "CommaSeparatedStrings::is_empty")]
    pub include: CommaSeparatedStrings,
}

impl QueryValidation for TransactionsQuery {
    fn validate(&self) -> anyhow::Result<()> {
        match (&self.sort.by, &self.filter) {
            (_, None) => (),
            (TransactionsQuerySortBy::Id, Some(TransactionsQueryFilter::Number(_))) => (),
            _ => anyhow::bail!("You can only filter and sort by the same field"),
        }

        self.pagination.validate()?;
        self.sort.validate()?;

        Ok(())
    }
}

impl TransactionsQuery {
    fn default_sorting() -> Sorting<TransactionsQuerySortBy> {
        Sorting {
            by: TransactionsQuerySortBy::Id,
            order: SortingOrder::Descending,
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "camelCase")]
pub enum TransactionsQuerySortBy {
    Id,
}

impl FromStr for TransactionsQuerySortBy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "id" => Ok(Self::Id),
            _ => anyhow::bail!("Invalid sort field, only 'id' is supported"),
        }
    }
}
