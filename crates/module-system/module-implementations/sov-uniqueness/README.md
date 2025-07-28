# `sov-uniqueness` module

The `sov-uniqueness` module is responsible for ensuring transaction deduplication on the rollup.

The module does not expose any `CallMessage` therefore, its state can't be directly modified by the users of the rollup. Instead the nonces/generation buckets are modified via the rollup's capabilities. 

Transaction deduplication can be done in two ways:
- Nonce deduplication: Each transaction sent by a given `sov_rollup_interface::crypto::CredentialId` has a unique nonce. This is a simple way to deduplicate transaction, this mechanism is similar to what is used by blockchains such as Ethereum. It is not possible to send a transaction with the same nonce twice, and the nonce is incremented by one for each transaction.
- Generation deduplication: Each transaction sent by a given `sov_rollup_interface::crypto::CredentialId` has an associated generation number. Each generation is mapped to a bucket of transactions that deduplicate transactions by their hash. Each credential can store at most `MAX_STORED_TX_HASHES_PER_CREDENTIAL` in `PAST_TRANSACTION_GENERATIONS` generations. When a transaction land with a generation number that is higher than the highest known generation, the buckets older than `new_generation - PAST_TRANSACTION_GENERATIONS` are pruned. This mechanism allows fast and (somewhat) stateless transaction deduplication, for usage by market makers for example. Transaction buckets can be mapped to an increasing timestamp with second granularity for instance.