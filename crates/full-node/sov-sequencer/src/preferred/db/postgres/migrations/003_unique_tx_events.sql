-- Prevent duplicate event rows from retry logic (run_with_retries!) that crashes on replay.

-- Step 1: Remove duplicate transaction rows that may already exist from retry behavior.
-- For each (sequence_number, index_in_batch) group, keep only the row with the lowest event_id.
DELETE FROM events e
USING (
    SELECT sequence_number, index_in_batch, MIN(event_id) AS keep_id
    FROM events
    WHERE event_type = 'transaction'
    GROUP BY sequence_number, index_in_batch
    HAVING COUNT(*) > 1
) dupes
WHERE e.event_type = 'transaction'
  AND e.sequence_number = dupes.sequence_number
  AND e.index_in_batch = dupes.index_in_batch
  AND e.event_id <> dupes.keep_id;

-- Step 2: Remove duplicate non-transaction event rows.
-- For each (sequence_number, event_type) group, keep only the row with the lowest event_id.
DELETE FROM events e
USING (
    SELECT sequence_number, event_type, MIN(event_id) AS keep_id
    FROM events
    WHERE event_type IN ('batch_start', 'batch_end', 'new_proof')
    GROUP BY sequence_number, event_type
    HAVING COUNT(*) > 1
) dupes
WHERE e.event_type = dupes.event_type
  AND e.sequence_number = dupes.sequence_number
  AND e.event_id <> dupes.keep_id;

-- Step 3: Create unique indexes.
-- Transactions: one per (sequence_number, index_in_batch).
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_tx
    ON events (sequence_number, index_in_batch)
    WHERE event_type = 'transaction';

-- Batch start/end and proofs: one per (sequence_number, event_type).
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_non_tx
    ON events (sequence_number, event_type)
    WHERE event_type IN ('batch_start', 'batch_end', 'new_proof');
