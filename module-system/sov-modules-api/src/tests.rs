use borsh::{BorshDeserialize, BorshSerialize};

use crate::default_signature::private_key::DefaultPrivateKey;
use crate::default_signature::{DefaultPublicKey, DefaultSignature};
use crate::Signature;

#[test]
fn test_account_bech32m_display() {
    let expected_addr: Vec<u8> = (1..=32).collect();
    let account = crate::AddressBech32::try_from(expected_addr.as_slice()).unwrap();
    assert_eq!(
        account.to_string(),
        "sov1qypqxpq9qcrsszg2pvxq6rs0zqg3yyc5z5tpwxqergd3c8g7rusqqsn6hm"
    );
}

#[test]
fn test_pub_key_serialization() {
    let pub_key = DefaultPrivateKey::generate().pub_key();
    let serialized_pub_key = pub_key.try_to_vec().unwrap();

    let deserialized_pub_key = DefaultPublicKey::try_from_slice(&serialized_pub_key).unwrap();
    assert_eq!(pub_key, deserialized_pub_key)
}

#[test]
fn test_signature_serialization() {
    let msg = [1; 32];
    let priv_key = DefaultPrivateKey::generate();

    let sig = priv_key.sign(msg);
    let serialized_sig = sig.try_to_vec().unwrap();
    let deserialized_sig = DefaultSignature::try_from_slice(&serialized_sig).unwrap();
    assert_eq!(sig, deserialized_sig);

    let pub_key = priv_key.pub_key();
    deserialized_sig.verify(&pub_key, msg).unwrap()
}

#[test]
fn test_hex_conversion() {
    let priv_key = DefaultPrivateKey::generate();
    let hex = priv_key.as_hex();
    let deserialized_pub_key = DefaultPrivateKey::from_hex(&hex).unwrap().pub_key();
    assert_eq!(priv_key.pub_key(), deserialized_pub_key)
}
