# Preferred Sequencer

## Rules for Update State (as of 2025/06/24)

Under normal operation, the preferred sequencer is continuously accepting new transactions. One of its most important jobs is to segment this stream of transactions into discrete batches. This is done according to a few rules.

1. We always try to start a new batch in response to `accept_tx` if there isn't already an active one
2. We only start a new batch if we have an unused `finalized_slot` to build on.

The sequencer can decide to close the active a batch for two different reasons:
1. It has successfully run `update_state` after receiving a new block from the node AND it is needs to increment the visible slot to close the gap between the visible slot and the latest finalized slot to the desired value OR
2.  The current batch is full


### Update State
The first of these paths is the most common "happy path" under steady state operation. In this path, the long running "state updater" background task hears from the node that a new block has been received and processed by 
the node. In response, the state updater creates a new block executor and uses it to replay all soft-confirmed transactions on top of the new checkpoint received from the node. Once the transactions have all been replayed, the new block executor has identical state to the old one, except that its view of the kernel state is fresher. At this point, we swap the new executor in for the old one and update seamlessly to the new version. 

Once the update is done, the state updater makes a final check to see if the current visible slot number (according to the *new* executor) is far enough behind the true slot number that we want to close out our current batch and start a new one (which increases the visible slot number). If so, we do exactly that.

### Accept Tx
The other path for closing the current batch is to recognize that the current batch is at capacity on some important dimension. (This could mean that it's reached its size limit (in bytes) its gas limit, or an execution time limit). Because `accept_tx` is now allowed to close batches on its own, the `update_state` process must handle the case in which the in-progress batch changes during transaction replay. 
This is done by subscribing to an event stream from the sequencer DB. Each time the sequencer accepts a tx or opens/closes a batch, it pushes an event
to this stream. 

To handle concurrency, the state update background task does the following:
1. Lock the sequencer
2. Clone the in-memory batches from the sequencer (i.e. the completed batches and the current in progress batch)
3. Subscribe to the event stream
4. (After dropping the lock) replay the batches
5. Drain any events from the stream and replay them as well until caught up.
