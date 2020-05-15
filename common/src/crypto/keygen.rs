use crate::config::ED25519Key;
use ring::signature;
use ring::signature::KeyPair;
use std::collections::BTreeMap;

/// Generate an ed25519 key pair.
///
/// Returns (private_key pkcs8 encoded , public_key)
pub fn generate_ed25519_key_pair() -> Result<(Vec<u8>, Vec<u8>), ring::error::Unspecified> {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8_bytes = signature::Ed25519KeyPair::generate_pkcs8(&rng)?;
    let key_pair = signature::Ed25519KeyPair::from_pkcs8(pkcs8_bytes.as_ref())?;
    let public_key = key_pair.public_key().as_ref().to_vec();
    Ok((pkcs8_bytes.as_ref().to_vec(), public_key))
}

pub fn generate_base64_encoded_keys(key_name: &str) -> (ED25519Key, BTreeMap<String, String>) {
    let (priv_key, pub_key) = generate_ed25519_key_pair().unwrap();
    let authorized_keys = vec![(key_name.to_string(), base64::encode(&pub_key))]
        .into_iter()
        .collect();
    (
        ED25519Key {
            id: key_name.to_string(),
            pkcs8: base64::encode(&priv_key),
            public_key: Some(base64::encode(&pub_key)),
        },
        authorized_keys,
    )
}
