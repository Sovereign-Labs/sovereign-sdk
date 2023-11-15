use sov_modules_core::{Prefix, StateCodec, StateReaderAndWriter, StateValueCodec};
use thiserror::Error;

/// Error type for `StateValue` get method.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

// StateReaderAndWriter
pub trait StateValueAccessor<V, Codec, W>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateValueCodec<V>,
    W: StateReaderAndWriter,
{
    /// Returns the prefix used when this [`StateValue`] was created.
    fn prefix(&self) -> &Prefix;

    fn codec(&self) -> &Codec;

    /// Sets a value in the StateValue.
    fn set(&self, value: &V, working_set: &mut W) {
        working_set.set_singleton(self.prefix(), value, self.codec())
    }

    /// Gets a value from the StateValue or None if the value is absent.
    fn get(&self, working_set: &mut W) -> Option<V> {
        working_set.get_singleton(self.prefix(), self.codec())
    }

    /// Gets a value from the StateValue or Error if the value is absent.
    fn get_or_err(&self, working_set: &mut W) -> Result<V, Error> {
        self.get(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Removes a value from the StateValue, returning the value (or None if the key is absent).
    fn remove(&self, working_set: &mut W) -> Option<V> {
        working_set.remove_singleton(self.prefix(), self.codec())
    }

    /// Removes a value and from the StateValue, returning the value (or Error if the key is absent).
    fn remove_or_err(&self, working_set: &mut W) -> Result<V, Error> {
        self.remove(working_set)
            .ok_or_else(|| Error::MissingValue(self.prefix().clone()))
    }

    /// Deletes a value from the StateValue.
    fn delete(&self, working_set: &mut W) {
        working_set.delete_singleton(self.prefix());
    }
}
