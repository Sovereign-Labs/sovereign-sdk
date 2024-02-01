use sov_modules_core::{Prefix, StateCodec, StateReaderAndWriter, StateValueCodec};
use thiserror::Error;

/// Error type for getters from state values method.
#[derive(Debug, Error)]
pub enum StateValueError {
    #[error("Value not found for prefix: {0}")]
    MissingValue(Prefix),
}

/// Allows a type to access a single value stored in the state.
pub trait StateValueAccessor<V, Codec, W>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateValueCodec<V>,
    W: StateReaderAndWriter,
{
    /// Returns the prefix used when this value was created.
    fn prefix(&self) -> &Prefix;

    /// Returns the codec used for this value
    fn codec(&self) -> &Codec;

    /// Sets the value.
    fn set(&self, value: &V, working_set: &mut W) {
        working_set.set_singleton(self.prefix(), value, self.codec())
    }

    /// Gets the value from state or returns None if the value is absent.
    fn get(&self, working_set: &mut W) -> Option<V> {
        working_set.get_singleton(self.prefix(), self.codec())
    }

    /// Gets the value from state or Error if the value is absent.
    fn get_or_err(&self, working_set: &mut W) -> Result<V, StateValueError> {
        self.get(working_set)
            .ok_or_else(|| StateValueError::MissingValue(self.prefix().clone()))
    }

    /// Removes the value from state, returning the value (or None if the key is absent).
    fn remove(&self, working_set: &mut W) -> Option<V> {
        working_set.remove_singleton(self.prefix(), self.codec())
    }

    /// Removes a value from state, returning the value (or Error if the key is absent).
    fn remove_or_err(&self, working_set: &mut W) -> Result<V, StateValueError> {
        self.remove(working_set)
            .ok_or_else(|| StateValueError::MissingValue(self.prefix().clone()))
    }

    /// Deletes a value from state.
    fn delete(&self, working_set: &mut W) {
        working_set.delete_singleton(self.prefix());
    }
}
