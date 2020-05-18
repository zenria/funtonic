use crate::config::ED25519Key;
use crate::crypto::signed_payload::payload_bytes_to_sign;
use chrono::{DateTime, Local};
use grpc_service::grpc_protocol::streaming_payload::Payload;
use grpc_service::payload::SignedPayload;
use prost::bytes;
use rand::random;
use ring::signature;
use ring::signature::KeyPair;
use rustbreak::deser::Yaml;
use rustbreak::FileDatabase;
use std::borrow::Borrow;
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::Path;
use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime};
use thiserror::Error;
use tonic::Status;

#[derive(Error, Debug)]
pub enum KeyStoreError {
    #[error("Key {0} does not exists")]
    KeyNotFound(String),
    #[error("Provided signature cannot be verified with key {0}")]
    WrongSignature(String),
    #[error("Signature expired on {0}, system time: {1}")]
    ExpiredSignature(String, String),
    #[error("Cannot decode payload: {0}")]
    PayloadDecodeError(String),
    #[error("Wrong key encoding {0}")]
    KeyEncodingError(#[from] base64::DecodeError),
    #[error("IOError {0}")]
    IOError(#[from] io::Error),
    #[error("Internal storage error {0}")]
    InternalStorage(#[from] rustbreak::RustbreakError),
    #[error("Poisonned lock (not possible AFAIK)")]
    Poison,
}

impl From<KeyStoreError> for Status {
    fn from(e: KeyStoreError) -> Self {
        Status::internal(e.to_string())
    }
}

pub trait KeyStoreBackend: Sized {
    fn insert_key<S: Into<String>>(
        &self,
        key_id: S,
        key_bytes: Vec<u8>,
    ) -> Result<(), KeyStoreError>;

    fn verify(&self, key_id: &str, payload: &[u8], signature: &[u8]) -> Result<(), KeyStoreError>;

    fn list_all(&self) -> Result<HashMap<String, Vec<u8>>, KeyStoreError>;

    fn remove_key(&self, key_id: &str) -> Result<Vec<u8>, KeyStoreError>;

    fn has_key(&self, key_id: &str, key_bytes: &[u8]) -> Result<bool, KeyStoreError>;
}

pub type FileKeyStoreBackend = FileDatabase<HashMap<String, Vec<u8>>, Yaml>;
pub type MemoryKeyStoreBackend = RwLock<HashMap<String, Vec<u8>>>;

fn verify_signature(
    db: &HashMap<String, Vec<u8>>,
    key_id: &str,
    payload: &[u8],
    signature: &[u8],
) -> Result<(), KeyStoreError> {
    db.get(key_id)
        .ok_or(KeyStoreError::KeyNotFound(key_id.to_string()))
        .and_then(|key_bytes| {
            signature::UnparsedPublicKey::new(&signature::ED25519, key_bytes)
                .verify(&payload, &signature)
                .map_err(|_| KeyStoreError::WrongSignature(key_id.to_string()))
        })
}

impl KeyStoreBackend for MemoryKeyStoreBackend {
    fn insert_key<S: Into<String>>(
        &self,
        key_id: S,
        key_bytes: Vec<u8>,
    ) -> Result<(), KeyStoreError> {
        self.write()
            .map_err(|_| KeyStoreError::Poison)?
            .insert(key_id.into(), key_bytes);
        Ok(())
    }

    fn verify(&self, key_id: &str, payload: &[u8], signature: &[u8]) -> Result<(), KeyStoreError> {
        verify_signature(
            self.read().map_err(|_| KeyStoreError::Poison)?.borrow(),
            key_id,
            payload,
            signature,
        )
    }

    fn list_all(&self) -> Result<HashMap<String, Vec<u8>, RandomState>, KeyStoreError> {
        Ok(self.read().map_err(|_| KeyStoreError::Poison)?.clone())
    }

    fn remove_key(&self, key_id: &str) -> Result<Vec<u8>, KeyStoreError> {
        self.write()
            .map_err(|_| KeyStoreError::Poison)?
            .remove(key_id)
            .ok_or(KeyStoreError::KeyNotFound(key_id.to_string()))
    }

    fn has_key(&self, key_id: &str, key_bytes: &[u8]) -> Result<bool, KeyStoreError> {
        Ok(self
            .read()
            .map_err(|_| KeyStoreError::Poison)?
            .get(key_id)
            .filter(|bytes| bytes.as_slice() == key_bytes)
            .is_some())
    }
}

impl KeyStoreBackend for FileKeyStoreBackend {
    fn insert_key<S: Into<String>>(
        &self,
        key_id: S,
        key_bytes: Vec<u8>,
    ) -> Result<(), KeyStoreError> {
        self.write(|db| {
            db.insert(key_id.into(), key_bytes);
        })?;
        Ok(self.save()?)
    }

    fn verify(&self, key_id: &str, payload: &[u8], signature: &[u8]) -> Result<(), KeyStoreError> {
        self.read(|db| verify_signature(db, key_id, payload, signature))?
    }

    fn list_all(&self) -> Result<HashMap<String, Vec<u8>>, KeyStoreError> {
        Ok(self.read(|db| db.clone())?)
    }

    fn remove_key(&self, key_id: &str) -> Result<Vec<u8>, KeyStoreError> {
        self.write(|db| {
            db.remove(key_id)
                .ok_or(KeyStoreError::KeyNotFound(key_id.to_string()))
        })?
        .and_then(|removed| {
            self.save()?;
            Ok(removed)
        })
    }

    fn has_key(&self, key_id: &str, key_bytes: &[u8]) -> Result<bool, KeyStoreError> {
        Ok(self.read(|db| {
            db.get(key_id)
                .filter(|bytes| bytes.as_slice() == key_bytes)
                .is_some()
        })?)
    }
}

/// Store ED25519 public key
pub struct KeyStore<B: KeyStoreBackend> {
    keys: B,
}

pub fn memory_keystore() -> KeyStore<MemoryKeyStoreBackend> {
    KeyStore {
        keys: Default::default(),
    }
}

pub fn file_keystore<P: AsRef<Path>>(
    path: P,
) -> Result<KeyStore<FileKeyStoreBackend>, KeyStoreError> {
    let path: &Path = path.as_ref();
    let initialize_db = !path.exists();
    let db = FileDatabase::<_, Yaml>::from_path(path, Default::default())?;
    if initialize_db {
        db.save()?;
    } else {
        db.load()?;
    }
    Ok(KeyStore { keys: db })
}

impl<B: KeyStoreBackend> KeyStore<B> {
    pub fn init_from_map<'a, T: IntoIterator<Item = (&'a String, &'a String)>>(
        self,
        map: T,
    ) -> Result<Self, KeyStoreError> {
        map.into_iter().try_fold(
            self,
            |store, (key, base64_encoded_bytes): (&String, &String)| {
                store.register_key(key, base64::decode(base64_encoded_bytes)?)?;
                Ok(store)
            },
        )
    }

    pub fn register_key<S: Into<String>>(
        &self,
        key_id: S,
        key_bytes: Vec<u8>,
    ) -> Result<(), KeyStoreError> {
        self.keys.insert_key(key_id.into(), key_bytes)
    }

    pub fn remove_key(&self, key_id: &str) -> Result<Vec<u8>, KeyStoreError> {
        self.keys.remove_key(key_id)
    }

    pub fn has_key(&self, key_id: &str, key_bytes: &[u8]) -> Result<bool, KeyStoreError> {
        self.keys.has_key(key_id, key_bytes)
    }

    pub fn decode_payload<P: prost::Message + Default>(
        &self,
        payload: &SignedPayload,
    ) -> Result<P, KeyStoreError> {
        // validate time limit
        let valid_until = SystemTime::UNIX_EPOCH + Duration::from_secs(payload.valid_until_secs);
        if valid_until < SystemTime::now() {
            Err(KeyStoreError::ExpiredSignature(
                DateTime::<Local>::from(valid_until).to_string(),
                DateTime::<Local>::from(SystemTime::now()).to_string(),
            ))?;
        }

        // check signature
        self.keys.verify(
            &payload.key_id,
            &payload_bytes_to_sign(&payload),
            &payload.signature,
        )?;

        // decode payload
        P::decode(payload.payload.as_slice())
            .map_err(|decode_err| KeyStoreError::PayloadDecodeError(decode_err.to_string()))
    }

    pub fn list_all(&self) -> Result<BTreeMap<String, String>, KeyStoreError> {
        self.keys.list_all().map(|keys| {
            keys.into_iter()
                .map(|(id, bytes)| (id, base64::encode(bytes)))
                .collect()
        })
    }
}
