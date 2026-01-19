-- Create table for tracking registered nodes
-- Each node registers itself with its address and maintains a heartbeat
CREATE TABLE IF NOT EXISTS nodes (
    node_id TEXT PRIMARY KEY,
    address TEXT NOT NULL,
    last_updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create index on last_updated for efficient queries on stale nodes
CREATE INDEX IF NOT EXISTS idx_nodes_last_updated ON nodes(last_updated);
