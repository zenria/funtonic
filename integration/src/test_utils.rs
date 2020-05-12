use commander::AdminCommandOuputMode;
use funtonic::config::Role::Commander;
use funtonic::config::{
    CommanderConfig, Config, ED25519Key, ExecutorConfig, Role, ServerConfig, TlsConfig,
};
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

pub fn generate_valid_keys(key_name: &str) -> (ED25519Key, BTreeMap<String, String>) {
    let (priv_key, pub_key) = generate_ed25519_key_pair().unwrap();
    let authorized_keys = vec![(key_name.to_string(), base64::encode(&pub_key))]
        .into_iter()
        .collect();
    (
        ED25519Key {
            id: key_name.to_string(),
            pkcs8: base64::encode(&priv_key),
            public_key: None,
        },
        authorized_keys,
    )
}

pub fn run_cmd_opt(query: &str, command: &str) -> commander::Opt {
    commander::Opt {
        config: None,
        command: commander::Command::Run(commander::cmd::Cmd {
            raw: false,
            group: false,
            no_progress: false,
            query: query.to_string(),
            command: vec![command.into()],
            no_std_process_return: true,
        }),
    }
}

pub fn admin_cmd() -> commander::Opt {
    commander::Opt {
        config: None,
        command: commander::Command::Admin {
            output_mode: AdminCommandOuputMode::Json,
            command: commander::AdminCommand::ListConnectedExecutors { query: None },
        },
    }
}

pub fn taskserver_config(
    port: u16,
    with_tls: bool,
    authorized_keys: BTreeMap<String, String>,
    admin_authorized_keys: BTreeMap<String, String>,
) -> Config {
    Config {
        tls: if with_tls {
            Some(TlsConfig {
                ca_cert: "tls/funtonic-ca.pem".to_string(),
                key: "tls/server-key.pem".to_string(),
                cert: "tls/server.pem".to_string(),
                server_domain: None,
            })
        } else {
            None
        },
        role: Role::Server(ServerConfig {
            bind_address: format!("127.0.0.1:{}", port),
            data_directory: "/tmp/taskserver".to_string(),
            authorized_keys,
            admin_authorized_keys,
        }),
    }
}

pub fn executor_config(
    port: u16,
    with_tls: bool,
    authorized_keys: BTreeMap<String, String>,
) -> Config {
    Config {
        tls: if with_tls {
            Some(TlsConfig {
                ca_cert: "tls/funtonic-ca.pem".to_string(),
                key: "tls/executor-key.pem".to_string(),
                cert: "tls/executor.pem".to_string(),
                server_domain: Some("test.funtonic.io".into()),
            })
        } else {
            None
        },
        role: Role::Executor(ExecutorConfig {
            client_id: "exec".to_string(),
            tags: Default::default(),
            server_url: format!("http://127.0.0.1:{}", port),
            authorized_keys,
        }),
    }
}

pub fn commander_config(port: u16, with_tls: bool, ed25519_key: ED25519Key) -> Config {
    Config {
        tls: if with_tls {
            Some(TlsConfig {
                ca_cert: "tls/funtonic-ca.pem".to_string(),
                key: "tls/commander-key.pem".to_string(),
                cert: "tls/commander.pem".to_string(),
                server_domain: Some("test.funtonic.io".into()),
            })
        } else {
            None
        },
        role: Commander(CommanderConfig {
            server_url: format!("http://127.0.0.1:{}", port),
            ed25519_key,
        }),
    }
}
