/// Generate the header that needs to be pre-pended to the message when serializing according to
/// the Ledger-compatible spec.
pub fn make_preamble_for_message(
    pubkey: &[u8; 32],
    chain_hash: &[u8; 32],
    message_length: u16,
) -> [u8; 85] {
    let mut header = Vec::<u8>::new();
    // Signing domain (pre-defined constant)
    header.extend(b"\xffsolana offchain");
    // Header version (only 0 is valid)
    header.push(0);
    // Application domain
    header.extend(chain_hash);
    // Message format - 0 is for ASCII, hardware wallet compatible
    header.push(0);
    // Signer count
    header.push(1);
    header.extend(pubkey);
    // Message length as little-endian u16
    header.extend(&message_length.to_le_bytes());
    header.try_into().unwrap()
}

/// Generate the header for a multisig spec-compliant message with multiple signers.
pub fn make_multisig_preamble_for_message(
    pubkeys: &[[u8; 32]],
    chain_hash: &[u8; 32],
    message_length: u16,
) -> Vec<u8> {
    use crate::authentication::{PREAMBLE_FIXED_LEN, PUBKEY_LEN};
    assert!(
        pubkeys.len() >= 2 && pubkeys.len() <= 255,
        "multisig preamble requires 2..=255 signers"
    );
    let mut header = Vec::with_capacity(PREAMBLE_FIXED_LEN + PUBKEY_LEN * pubkeys.len());
    // Signing domain (pre-defined constant)
    header.extend(b"\xffsolana offchain");
    // Header version (only 0 is valid)
    header.push(0);
    // Application domain
    header.extend(chain_hash);
    // Message format - 0 is for ASCII, hardware wallet compatible
    header.push(0);
    // Signer count
    header.push(pubkeys.len() as u8);
    for pubkey in pubkeys {
        header.extend(pubkey);
    }
    // Message length as little-endian u16
    header.extend(&message_length.to_le_bytes());
    header
}
