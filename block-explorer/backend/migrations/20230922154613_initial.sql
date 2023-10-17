CREATE TABLE blocks (
	id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
	blob JSONB NOT NULL
);

CREATE INDEX blocks_height_idx ON blocks ((blob ->> 'height'));

CREATE UNIQUE INDEX blocks_hash_idx ON blocks ((blob ->> 'hash'));

CREATE TABLE transactions (
	id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
	blob JSONB NOT NULL
);

CREATE UNIQUE INDEX transactions_hash_idx ON transactions ((blob ->> 'hash'));

CREATE TABLE block_transactions (
	block_id BIGINT NOT NULL REFERENCES blocks (id),
	transaction_id BIGINT NOT NULL REFERENCES transactions (id),
	PRIMARY KEY (block_id, transaction_id)
);

CREATE INDEX block_transactions_tx_id_idx ON block_transactions (transaction_id);
