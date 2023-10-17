CREATE TABLE batches (
	id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
	blob JSONB NOT NULL
);

CREATE UNIQUE INDEX batches_hash_idx ON batches ((blob ->> 'hash'));
