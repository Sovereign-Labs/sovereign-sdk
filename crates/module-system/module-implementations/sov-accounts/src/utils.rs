use sov_modules_api::digest::Digest;
use sov_modules_api::{CredentialId, CryptoSpec, Spec};

/// Derives the address assigned to a newly inserted credential.
///
/// The derivation mixes the transaction sender into the hash so that:
/// - a multisig registered by A does not alias A's own address (a later
///   compromise of A's single key must not also control the multisig); and
/// - if credential removal is ever added, a removed credential cannot be
///   re-registered by a different sender and collide with the old address.
pub fn derive_address_for_new_credential<S: Spec>(
    new_credential_id: &CredentialId,
    sender: &S::Address,
) -> S::Address {
    let mut hasher = <S::CryptoSpec as CryptoSpec>::Hasher::new();
    hasher.update(new_credential_id.0 .0);
    hasher.update(sender.as_ref());
    let hash: [u8; 32] = hasher.finalize().into();
    // Route through `CredentialId` so each Spec's existing `From<CredentialId>`
    // handles address-width reduction (28 / 32 / 20 bytes).
    S::Address::from(CredentialId::from_bytes(hash))
}
