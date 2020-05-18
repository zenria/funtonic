pub mod keygen;
pub mod keystore;
pub mod signed_payload;

#[cfg(test)]
mod test {
    use crate::crypto::keygen::generate_ed25519_key_pair;
    use crate::crypto::keystore::{file_keystore, memory_keystore};
    use crate::crypto::signed_payload::encode_and_sign;
    use crate::path_builder::PathBuilder;
    use prost::Message;
    use ring::signature;
    use ring::signature::KeyPair;
    use std::fs::{read_to_string, File};
    use std::path::PathBuf;
    use tokio::time::Duration;

    #[derive(Clone, PartialEq, Message)]
    struct TestPayload {
        #[prost(string, tag = "1")]
        some_stuff: String,
    }
    #[test]
    fn test() {
        let (private_key, public_key) = generate_ed25519_key_pair().unwrap();

        let key_store = memory_keystore();
        key_store.register_key("abcd", public_key.to_vec()).unwrap();

        let payload = TestPayload {
            some_stuff: "foo // bar".into(),
        };

        let signed_payload = encode_and_sign(
            payload,
            &("abcd", private_key.as_slice()).into(),
            Duration::from_secs(5),
        )
        .unwrap();

        let decoded = key_store
            .decode_payload::<TestPayload>(&signed_payload)
            .unwrap();
        assert_eq!(&decoded.some_stuff, "foo // bar");
    }
    #[test]
    fn test_filebacked_keystore() {
        let dir = tempfile::tempdir().unwrap();
        let file = PathBuilder::from_path(&dir).push("keystore.yaml").build();
        assert!(!file.exists());

        let (private_key, public_key) = generate_ed25519_key_pair().unwrap();

        let payload = TestPayload {
            some_stuff: "foo // bar".into(),
        };

        let signed_payload = encode_and_sign(
            payload,
            &("abcd", private_key.as_slice()).into(),
            Duration::from_secs(5),
        )
        .unwrap();

        {
            let ks = file_keystore(&file).unwrap();
            assert!(file.exists());
            ks.register_key("abcd", public_key.to_vec()).unwrap();

            let decoded = ks.decode_payload::<TestPayload>(&signed_payload).unwrap();
            assert_eq!(&decoded.some_stuff, "foo // bar");
        }
        {
            // the keystore should be persisted on the filesystem: no need to register a key anymore
            assert!(file.exists());
            let ks = file_keystore(&file).unwrap();

            let decoded = ks.decode_payload::<TestPayload>(&signed_payload).unwrap();
            assert_eq!(&decoded.some_stuff, "foo // bar");
        }
    }
}
