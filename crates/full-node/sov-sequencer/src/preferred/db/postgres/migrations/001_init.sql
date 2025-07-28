-- Create enum for event types
CREATE TYPE event_type AS ENUM ('transaction', 'batch_start', 'batch_end', 'new_proof');

-- Create events table with proper event-driven structure
CREATE TABLE IF NOT EXISTS events (
	event_id BIGSERIAL PRIMARY KEY,
	sequence_number BIGINT NOT NULL CHECK (sequence_number >= 0),
	event_type event_type NOT NULL,
	index_in_batch BIGINT CHECK (index_in_batch >= 0 OR index_in_batch IS NULL),
	hash BYTEA,
	data BYTEA
);

-- Add check constraints to ensure data integrity according to event type
ALTER TABLE events ADD CONSTRAINT events_transaction_data_integrity 
CHECK (
    (event_type = 'transaction' AND index_in_batch >= 0 AND hash IS NOT NULL AND data IS NOT NULL) OR -- Transactions have both hash and data
    (event_type IN ('batch_start', 'batch_end') AND hash IS NULL AND data IS NOT NULL) OR -- Batches don't store a hash
    (event_type = 'new_proof' AND hash IS NULL AND data IS NULL) -- Proofs are stored in another table
);

-- Ensure index_in_batch is meaningful only for transactions
ALTER TABLE events ADD CONSTRAINT events_transaction_index_validity
CHECK (
    (event_type = 'transaction' AND index_in_batch >= 0) OR
    (event_type IN ('batch_start', 'batch_end', 'new_proof') AND index_in_batch IS NULL)
);

CREATE TABLE IF NOT EXISTS proof_blobs (
	sequence_number BIGINT PRIMARY KEY CHECK (sequence_number >= 0),
	borsh_value BYTEA NOT NULL
);

CREATE TABLE IF NOT EXISTS in_progress_batch (
	-- See <https://stackoverflow.com/a/72358001>.
	singleton INTEGER GENERATED ALWAYS AS (1) STORED UNIQUE,
	sequence_number BIGINT NOT NULL CHECK (sequence_number >= 0),
	borsh_value BYTEA NOT NULL
);

-- Create NOTIFY trigger for events table changes
CREATE OR REPLACE FUNCTION notify_events_changes()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('events_changes',
        NEW.event_id::text || ',' ||
        NEW.sequence_number::text || ',' ||
        NEW.event_type::text || ',' ||
        COALESCE(NEW.index_in_batch::text, '')
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER events_changes_trigger
    AFTER INSERT ON events
    FOR EACH ROW
    EXECUTE FUNCTION notify_events_changes();
