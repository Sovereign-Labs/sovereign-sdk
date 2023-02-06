pub enum WriteOp<K, V> {
    Delete(K),
    Insert(K, V),
}

/// An EphemeralStateWriter can update keys and values but not persist them
pub trait EphemeralStateWriter {
    type Key;
    type Value = Bytes;
    /// Store a key-value pair. Returns the previous value of `key`, if it existed.
    pub fn put(&mut self, key: Key, value: Value) -> Option<Value>;
    /// Delete a key from the store.  
    pub fn delete(&mut self, key: Key) -> Option<Value>;
}

/// A state commiter represents an immutable batch of operations to be applied to
/// a state DB en masse (or dropped).
pub trait StateCommitter {
    type Key;
    type Value = Bytes;
    /// Returns an iterator over the key-value pairs modified since the last commit
    pub fn change_set(&self) -> impl Iterator<Item = &WriteOp<Key, Value>>;
    /// Returns the op that will be written for the provieded key on commit
    pub fn get_op_for(&self, key: &Key) -> Option<&WriteOp<Key, Value>>;
    /// Commits the current change set
    pub fn commit(self);
}

/// A state writer can update keys and values. Changes must be `commit`ted before
/// they persist.
pub trait StateWriter:
    EphemeralStateWriter<Key = Self::Key, Value = Self::Value>
    + StateCommitter<Key = Self::Key, Value = Self::Value>
{
    type Key;
    type Value = Bytes;
    type Frozen: StateCommitter<Key = Self::Key, Value = Self::Value>;

    /// Freezes an update set, preventing further write ops against it.
    /// A frozen state writer may still be committed to the DB, but no additional changes
    /// may be made.
    pub fn freeze(self) -> Self::Frozen;
    /// Drops the current change set, reverting to the previous `commit`.
    pub fn reset(&mut self);
}

/// A state reader can get key-value pairs
pub trait StateReader {
    type Key;
    type Value = Bytes;
    pub fn get(&self, key: &Key) -> Option<&Value>;
}

/// Read, write, and commit key-value pairs
pub trait StateDb:
    StateWriter<Key = Self::Key, Value = Self::Value>
    + StateReader<Key = Self::Key, Value = Self::Value>
{
    type Key;
    type Value = Bytes;
}

/// A domain-separated stateDB
pub trait Keeper: StateDb {
    type Key;
    type Value = Bytes;
    fn with_prefix(prefix: Key);
}
