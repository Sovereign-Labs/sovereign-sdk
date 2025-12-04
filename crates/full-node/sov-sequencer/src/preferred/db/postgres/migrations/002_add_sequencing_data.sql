-- Add sequencing_data column to store sequencer-provided metadata (e.g., timestamps)
-- This data is NOT signed by users but is included for modules to access via Context
ALTER TABLE events
ADD COLUMN sequencing_data BYTEA;

-- Update the check constraint to allow sequencing_data for transactions
-- (It's optional, so can be NULL)
ALTER TABLE events DROP CONSTRAINT IF EXISTS events_transaction_data_integrity;
ALTER TABLE events ADD CONSTRAINT events_transaction_data_integrity
CHECK (
    (event_type = 'transaction' AND index_in_batch >= 0 AND hash IS NOT NULL AND data IS NOT NULL) OR
    (event_type IN ('batch_start', 'batch_end') AND hash IS NULL AND data IS NOT NULL) OR
    (event_type = 'new_proof' AND hash IS NULL AND data IS NULL)
);
