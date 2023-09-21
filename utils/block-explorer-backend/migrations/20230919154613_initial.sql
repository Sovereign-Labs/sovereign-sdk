CREATE TABLE blocks (
	id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
	hash BYTEA NOT NULL,
	height BIGINT NOT NULL,
	contents BYTEA NOT NULL
);

CREATE INDEX blocks_height_idx ON blocks (height);
CREATE INDEX blocks_hash_idx ON blocks (hash);

CREATE TABLE transactions (
	id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
	hash BYTEA NOT NULL,
	block_id BIGINT NOT NULL,
	contents BYTEA NOT NULL
);
