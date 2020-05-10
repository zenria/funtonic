use crate::signed_payload::DecodePayloadError::KeyNotFound;
use bytes::BytesMut;
use chrono::{DateTime, Local};
use grpc_service::grpc_protocol::streaming_payload::Payload;
use grpc_service::payload::SignedPayload;
use prost::bytes;
use rand::random;
use ring::signature;
use ring::signature::KeyPair;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DecodePayloadError {
    #[error("Key {0} does not exists")]
    KeyNotFound(String),
    #[error("Provided signature cannot be verified with key {0}")]
    WrongSignature(String),
    #[error("Signature expired on {0}, system time: {1}")]
    ExpiredSignature(String, String),
    #[error("Cannot decode payload: {0}")]
    PayloadDecodeError(String),
}

/// Store ED25519 public key
pub struct KeyStore {
    keys: HashMap<String, Vec<u8>>,
}

impl KeyStore {
    pub fn new() -> Self {
        Self {
            keys: Default::default(),
        }
    }

    pub fn register_key<S: Into<String>>(&mut self, key_id: S, key_bytes: &[u8]) {
        self.keys.insert(key_id.into(), key_bytes.to_vec());
    }

    pub fn decode_payload<P: prost::Message + Default>(
        &self,
        payload: SignedPayload,
    ) -> Result<P, DecodePayloadError> {
        // validate time limit
        let valid_until = SystemTime::UNIX_EPOCH + Duration::from_secs(payload.valid_until_secs);
        if valid_until < SystemTime::now() {
            Err(DecodePayloadError::ExpiredSignature(
                DateTime::<Local>::from(valid_until).to_string(),
                DateTime::<Local>::from(SystemTime::now()).to_string(),
            ))?;
        }

        // find key
        let key_bytes = self
            .keys
            .get(&payload.key_id)
            .ok_or(KeyNotFound(payload.key_id.clone()))?;

        // check signature
        let public_key = signature::UnparsedPublicKey::new(&signature::ED25519, key_bytes);
        public_key
            .verify(&payload_bytes_to_sign(&payload), &payload.signature)
            .map_err(|_| DecodePayloadError::WrongSignature(payload.key_id.clone()))?;

        P::decode(payload.payload.as_slice())
            .map_err(|decode_err| DecodePayloadError::PayloadDecodeError(decode_err.to_string()))
    }
}

fn payload_bytes_to_sign(payload: &SignedPayload) -> Vec<u8> {
    to_sign_from_exploded_payload(&payload.payload, payload.nonce, payload.valid_until_secs)
}

pub fn to_sign_from_exploded_payload(payload: &[u8], nonce: u64, valid_until_secs: u64) -> Vec<u8> {
    payload
        .iter() // Iter<Item=&u8>
        .chain(nonce.to_le_bytes().iter())
        .chain(valid_until_secs.to_le_bytes().iter())
        .map(|u8_ref| *u8_ref) // rust is a bit annoying
        .collect()
}

const BUFFER_SIZE: usize = 8 * 1024;

#[derive(Error, Debug)]
pub enum EncodePayloadError {
    #[error("Invalid key provided: {0}")]
    KeyRejected(String),
    #[error("Cannot encode message {0}")]
    EncodeError(String),
    #[error("Please adjust your system clock lol")]
    SystemClockIsBeforeUnixEpoch,
}

pub fn encode_and_sign<P: prost::Message>(
    payload: P,
    key_id: &str,
    pkcs8_key: &[u8],
    validity: Duration,
) -> Result<SignedPayload, EncodePayloadError> {
    let valid_until_secs = (SystemTime::now() + validity)
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| EncodePayloadError::SystemClockIsBeforeUnixEpoch)?
        .as_secs();
    let nonce = random();

    let key_pair = signature::Ed25519KeyPair::from_pkcs8(pkcs8_key)
        .map_err(|e| EncodePayloadError::KeyRejected(e.to_string()))?;

    let mut buf = BytesMut::with_capacity(BUFFER_SIZE);
    payload
        .encode(&mut buf)
        .map_err(|e| EncodePayloadError::EncodeError(e.to_string()))?;
    let payload = buf.to_vec();

    let signature = Vec::from(
        key_pair
            .sign(&to_sign_from_exploded_payload(
                &payload,
                nonce,
                valid_until_secs,
            ))
            .as_ref(),
    );
    Ok(SignedPayload {
        payload,
        nonce,
        valid_until_secs,
        signature,
        key_id: key_id.to_string(),
    })
}

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

#[cfg(test)]
mod test {
    use crate::signed_payload::{encode_and_sign, generate_ed25519_key_pair, KeyStore};
    use prost::Message;
    use ring::signature;
    use ring::signature::KeyPair;
    use tokio::time::Duration;

    #[derive(Clone, PartialEq, Message)]
    struct TestPayload {
        #[prost(string, tag = "1")]
        some_stuff: String,
    }
    #[test]
    fn test() {
        let (private_key, public_key) = generate_ed25519_key_pair().unwrap();

        let mut key_store = KeyStore::new();
        key_store.register_key("abcd", &public_key);

        let payload = TestPayload {
            some_stuff: "foo // bar".into(),
        };

        let signed_payload =
            encode_and_sign(payload, "abcd", &private_key, Duration::from_secs(5)).unwrap();

        let decoded = key_store
            .decode_payload::<TestPayload>(signed_payload)
            .unwrap();
        assert_eq!(&decoded.some_stuff, "foo // bar");
    }
}
