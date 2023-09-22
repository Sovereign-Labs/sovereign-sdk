# Schema DB

This package is a low-level wrapper transforming [RocksDB](https://rocksdb.org/) from a byte-oriented key value store into a
type-oriented store. It's adapted from a similar package in Aptos-Core.

The most important concept exposed by Schema DB is a `Schema`, which maps a column-family name
codec's for the key and value.

```rust
pub trait Schema  {
    /// The column family name associated with this struct.
    /// Note: all schemas within the same SchemaDB must have distinct column family names.
    const COLUMN_FAMILY_NAME: &'static str;

    /// Type of the key.
    type Key: KeyCodec<Self>;

    /// Type of the value.
    type Value: ValueCodec<Self>;
}
```

Using this schema, we can write generic functions for storing and fetching typed data, and ensure that
it's always encoded/decoded in the way we expect.

```rust
impl SchemaDB {
	pub fn put<S: Schema>(&self, key: &impl KeyCodec<S>, value: &impl ValueCodec<S>) -> Result<()> {
		let key_bytes = key.encode_key()?;
        let value_bytes = value.encode_value()?;
		self.rocks_db_handle.put(S::COLUMN_FAMILY_NAME, key_bytes, value_bytes)
	}
}
```

To actually store and retrieve data, all we need to do is to implement a Schema:

```rust
pub struct AccountBalanceSchema;

impl Schema for AccountBalanceSchema {
	const COLUMN_FAMILY_NAME: &str = "account_balances";
	type Key = Account;
	type Value = u64;
}

impl KeyCodec<AccountBalanceSchema> for Account {
	fn encode_key(&self) -> Vec<u8> {
		bincode::to_vec(self)
	}

	fn decode_key(key: Vec<u8>) -> Self {
		// elided
	}
}

impl ValueCode<AccountBlanceSchema> for u64 {
	// elided
}
```
