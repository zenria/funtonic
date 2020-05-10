#[cfg(test)]
mod test_utils;

#[cfg(test)]
mod tests {
    use commander::{cmd::Cmd, commander_main, AdminCommand, AdminCommandOuputMode, Command, Opt};
    use executor::executor_main;
    use funtonic::config::Role::Commander;
    use funtonic::config::{
        CommanderConfig, Config, ED25519Key, ExecutorConfig, Role, ServerConfig, TlsConfig,
    };
    use funtonic::signed_payload::generate_ed25519_key_pair;
    use log::LevelFilter;
    use std::collections::BTreeMap;
    use std::sync::Once;
    use std::time::Duration;
    use taskserver::taskserver_main;

    static INIT_LOGGER: Once = Once::new();

    fn init_logger() {
        INIT_LOGGER.call_once(|| env_logger::builder().filter_level(LevelFilter::Info).init())
    }

    #[tokio::test]
    async fn no_tls_test() {
        init_logger();

        let mut authorized_client_tokens = BTreeMap::new();
        authorized_client_tokens.insert("coucou".into(), "test client".into());
        let taskserver_config = Config {
            tls: None,
            role: Role::Server(ServerConfig {
                bind_address: "127.0.0.1:54010".to_string(),
                data_directory: "/tmp/taskserver".to_string(),
                authorized_client_tokens,
                authorized_keys: Default::default(),
                admin_authorized_keys: Default::default(),
            }),
        };

        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));

        let executor_config = Config {
            tls: None,
            role: Role::Executor(ExecutorConfig {
                client_id: "exec".to_string(),
                tags: Default::default(),
                server_url: "http://127.0.0.1:54010".to_string(),
            }),
        };
        super::test_utils::spawn_future_on_new_thread(|| executor_main(executor_config));

        let commander_config = Config {
            tls: None,
            role: Commander(CommanderConfig {
                server_url: "http://127.0.0.1:54010".to_string(),
                client_token: "coucou".to_string(),
                ed25519_key: ("abcd", "1234".as_bytes()).into(),
            }),
        };

        let commander_opt = Opt {
            config: None,
            command: Command::Run(Cmd {
                raw: false,
                group: false,
                no_progress: false,
                query: "*".to_string(),
                command: vec!["cat".into(), "Cargo.toml".into()],
                no_std_process_return: true,
            }),
        };
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config)
            .await
            .expect("This must not throw errors");
    }

    #[tokio::test]
    async fn tls_test() {
        init_logger();

        let (priv_key, pub_key) = generate_ed25519_key_pair().unwrap();
        let authorized_keys = vec![("foo_key".to_string(), base64::encode(&pub_key))]
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        let mut authorized_client_tokens = BTreeMap::new();
        authorized_client_tokens.insert("coucou".into(), "test client".into());
        let taskserver_config = Config {
            tls: Some(TlsConfig {
                ca_cert: "tls/funtonic-ca.pem".to_string(),
                key: "tls/server-key.pem".to_string(),
                cert: "tls/server.pem".to_string(),
                server_domain: None,
            }),
            role: Role::Server(ServerConfig {
                bind_address: "127.0.0.1:54011".to_string(),
                data_directory: "/tmp/taskserver".to_string(),
                authorized_keys: authorized_keys.clone(),
                admin_authorized_keys: authorized_keys.clone(),
                authorized_client_tokens,
            }),
        };

        super::test_utils::spawn_future_on_new_thread(|| taskserver_main(taskserver_config));

        let executor_config = Config {
            tls: Some(TlsConfig {
                ca_cert: "tls/funtonic-ca.pem".to_string(),
                key: "tls/executor-key.pem".to_string(),
                cert: "tls/executor.pem".to_string(),
                server_domain: Some("test.funtonic.io".into()),
            }),
            role: Role::Executor(ExecutorConfig {
                client_id: "exec".to_string(),
                tags: Default::default(),
                server_url: "http://127.0.0.1:54011".to_string(),
            }),
        };
        super::test_utils::spawn_future_on_new_thread(|| executor_main(executor_config));

        let commander_config = || Config {
            tls: Some(TlsConfig {
                ca_cert: "tls/funtonic-ca.pem".to_string(),
                key: "tls/commander-key.pem".to_string(),
                cert: "tls/commander.pem".to_string(),
                server_domain: Some("test.funtonic.io".into()),
            }),
            role: Commander(CommanderConfig {
                server_url: "http://127.0.0.1:54011".to_string(),
                client_token: "coucou".to_string(),
                ed25519_key: ED25519Key {
                    id: "foo_key".to_string(),
                    pkcs8: base64::encode(&priv_key),
                },
            }),
        };

        let commander_opt = Opt {
            config: None,
            command: Command::Run(Cmd {
                raw: false,
                group: false,
                no_progress: false,
                query: "*".to_string(),
                command: vec!["cat".into(), "Cargo.toml".into()],
                no_std_process_return: true,
            }),
        };
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config())
            .await
            .unwrap();

        let commander_opt = Opt {
            config: None,
            command: Command::Admin {
                output_mode: AdminCommandOuputMode::Json,
                command: AdminCommand::ListConnectedExecutors { query: None },
            },
        };
        std::thread::sleep(Duration::from_secs(1));
        commander_main(commander_opt, commander_config())
            .await
            .unwrap();
    }
}
