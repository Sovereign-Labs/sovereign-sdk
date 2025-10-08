use alloy::hex;

pub(crate) fn generate_priv_keys(count: usize, private_key: &str) -> anyhow::Result<Vec<String>> {
    if count > 256 {
        return Err(anyhow::anyhow!("count must be less than 255 because of our private key tweaking. This is an easy fix, but we haven't done it yet."));
    }

    let mut priv_keys = Vec::with_capacity(count);

    for i in 1..count + 1 {
        let key = {
            let mut key_bytes: [u8; 32] = hex::decode(private_key).unwrap().try_into().unwrap();
            key_bytes[0] = key_bytes[0].wrapping_add(i as u8);
            hex::encode(key_bytes)
        };
        priv_keys.push(key);
    }

    Ok(priv_keys)
}
