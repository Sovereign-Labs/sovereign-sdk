#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Event {
    pub id: i64,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
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

#[derive(Debug)]
pub struct HexString(pub Vec<u8>);

impl AsRef<Vec<u8>> for HexString {
    fn as_ref(&self) -> &Vec<u8> {
        &self.0
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
