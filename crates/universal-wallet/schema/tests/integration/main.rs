mod eip712;
mod schema;

// Hack - because the macro is configured to be re-exported from sov_rollup_interface;
// but _we_ are a dependency of sov_rollup_interface so we can't import it without causing a cycle
// This should not be an issue anywhere else except inside this crate's tests right here
mod sov_rollup_interface {
    pub use sov_universal_wallet;
}
