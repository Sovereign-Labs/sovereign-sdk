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
