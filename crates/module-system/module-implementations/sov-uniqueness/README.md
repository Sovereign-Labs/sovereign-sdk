# `sov-uniqueness` module

The `sov-uniqueness` module is responsible for ensuring transaction deduplication on the rollup.

The module does not expose any `CallMessage`, so users cannot directly modify its state. Instead, nonce, generation, and windowed nonce state is updated through rollup capabilities during transaction processing.

Transaction deduplication can be done in three ways:

- Nonce deduplication: Each transaction sent by a given `sov_rollup_interface::crypto::CredentialId` has a unique sequential nonce. This is similar to nonce handling in blockchains such as Ethereum. It is not possible to send a transaction with the same nonce twice, and the nonce is incremented by one for each transaction.
- Generation deduplication: Each transaction sent by a given credential has an associated generation number. Each generation is mapped to a bucket of transaction hashes. Each credential can store at most `MAX_STORED_TX_HASHES_PER_CREDENTIAL` hashes across `PAST_TRANSACTION_GENERATIONS` generations. When a transaction lands with a generation higher than the highest known generation, buckets older than `new_generation - PAST_TRANSACTION_GENERATIONS` are pruned. This mechanism allows fast and somewhat stateless deduplication; for example, transaction buckets can be mapped to an increasing timestamp with second granularity.
- Windowed nonce deduplication: Each transaction sent by a given credential has a nonce that must not have been seen before, but nonces do not need to be consecutive. The module stores a bitmap over the most recent `PAST_TRANSACTIONS_WINDOW` nonce range. Nonces below the current window are treated as already consumed, while unseen nonces inside or ahead of the window can be accepted.

For generation and windowed nonce deduplication, fresh credentials with no stored state may submit any initial value. After that, generation transactions may not increase the latest known generation above `max(latest * 2, latest + PAST_TRANSACTION_GENERATIONS + 1)`, which still permits the minimum jump needed to prune existing buckets. Windowed nonce transactions may not increase the highest seen nonce above `max(highest * 2, PAST_TRANSACTIONS_WINDOW)`. These limits prevent accidental jumps such as submitting millisecond timestamps where second timestamps were intended, or submitting `u64::MAX`, while still allowing intentional larger migrations through a short series of intermediate transactions. Values cannot be lowered again without reintroducing replay risks.
