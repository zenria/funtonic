use funtonic::config::ED25519Key;
use funtonic::signed_payload::generate_ed25519_key_pair;
use futures::Future;
use std::collections::BTreeMap;
use std::error::Error;

pub fn spawn_future_on_new_thread<
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn Error>>>,
>(
    f: F,
) {
    std::thread::spawn(move || {
        let mut rt = tokio::runtime::Builder::new()
            .basic_scheduler()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(f()).unwrap();
    });
}

pub fn generate_valid_keys() -> (ED25519Key, BTreeMap<String, String>) {
    let (priv_key, pub_key) = generate_ed25519_key_pair().unwrap();
    let authorized_keys = vec![("foobar_id".to_string(), base64::encode(&pub_key))]
        .into_iter()
        .collect();
    (
        ED25519Key {
            id: "foobar_id".to_string(),
            pkcs8: base64::encode(&priv_key),
            public_key: None,
        },
        authorized_keys,
    )
}
