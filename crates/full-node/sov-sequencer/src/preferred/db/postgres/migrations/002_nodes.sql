-- Create table for tracking registered nodes
-- Each node registers itself with its address and maintains a heartbeat
CREATE TABLE IF NOT EXISTS nodes (
    node_id TEXT PRIMARY KEY,
    address TEXT NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create index on last_updated for efficient queries on stale nodes
CREATE INDEX IF NOT EXISTS idx_nodes_last_updated ON nodes(last_updated);

-- Timestamp when leadership was acquired; used for grace-period checks
ALTER TABLE sequencer_leader
ADD COLUMN leader_acquired_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

-- Create NOTIFY trigger for nodes table changes
CREATE OR REPLACE FUNCTION notify_nodes_changes()
RETURNS TRIGGER AS $$
DECLARE
    payload TEXT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        payload := OLD.node_id || ',' || OLD.address || ',' || TG_OP;
        PERFORM pg_notify('nodes_changes', payload);
        RETURN OLD;
    ELSE
        payload := NEW.node_id || ',' || NEW.address || ',' || TG_OP;
        PERFORM pg_notify('nodes_changes', payload);
        RETURN NEW;
    END IF;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER nodes_changes_trigger
    AFTER INSERT OR UPDATE OR DELETE ON nodes
    FOR EACH ROW
    EXECUTE FUNCTION notify_nodes_changes();
