use crate::config::ED25519Key;
use bytes::BytesMut;
use grpc_service::payload::SignedPayload;
use rand::random;
use ring::signature;
use std::time::{Duration, SystemTime};
use thiserror::Error;

pub fn payload_bytes_to_sign(payload: &SignedPayload) -> Vec<u8> {
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
    key: &ED25519Key,
    validity: Duration,
) -> Result<SignedPayload, EncodePayloadError> {
    let valid_until_secs = (SystemTime::now() + validity)
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| EncodePayloadError::SystemClockIsBeforeUnixEpoch)?
        .as_secs();
    let nonce = random();

    let key_pair = signature::Ed25519KeyPair::from_pkcs8(
        &key.to_bytes()
            .map_err(|e| EncodePayloadError::KeyRejected(e.to_string()))?,
    )
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
        key_id: key.id().to_string(),
    })
}
