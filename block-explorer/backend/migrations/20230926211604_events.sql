CREATE TABLE events (
	id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
	key BYTEA NOT NULL,
	VALUE BYTEA NOT NULL
);

CREATE TABLE tx_events (
	tx_id BIGINT NOT NULL REFERENCES transactions (id),
	event_id BIGINT NOT NULL REFERENCES events (id),
	PRIMARY KEY (tx_id, event_id)
);

CREATE INDEX tx_events_event_id_idx ON tx_events (event_id);
