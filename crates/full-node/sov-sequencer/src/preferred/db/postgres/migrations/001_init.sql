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


-- Create table for sequencer leader election
-- This table maintains exactly one row containing the current leader node ID and heartbeat timestamp
CREATE TABLE IF NOT EXISTS sequencer_leader (
    -- Singleton constraint - only one row allowed in this table
    singleton INTEGER GENERATED ALWAYS AS (1) STORED UNIQUE,
    -- Node ID of the current leader
    node_id TEXT NOT NULL,
    -- Timestamp when the leader last sent a heartbeat
    last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Primary key on the singleton to enforce single row
    PRIMARY KEY (singleton)
);

-- Create NOTIFY trigger for leader election table changes
CREATE OR REPLACE FUNCTION notify_leader_changes()
RETURNS TRIGGER AS $$
BEGIN
    -- Send notification with node_id and timestamp for INSERT and UPDATE operations
    PERFORM pg_notify('leader_changes',
        NEW.node_id::text || ',' ||
        EXTRACT(EPOCH FROM NEW.last_updated)::text
    );
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER leader_changes_trigger
    AFTER INSERT OR UPDATE ON sequencer_leader
    FOR EACH ROW
    EXECUTE FUNCTION notify_leader_changes();


CREATE FUNCTION is_leader(p_node_id TEXT)
RETURNS boolean AS $$
    SELECT EXISTS (
        SELECT 1
        FROM sequencer_leader
        WHERE singleton = 1 AND node_id = p_node_id
    );
$$ LANGUAGE sql STABLE;