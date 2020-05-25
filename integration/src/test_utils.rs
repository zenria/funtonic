use commander::cmd::CommandOptions;
use commander::{AdminCommandOuputMode, CommanderSyntheticOutput, ExecutorState};
use executor::executor_main;
use funtonic::config::{CommanderConfig, ED25519Key, ExecutorConfig, ServerConfig, TlsConfig};
use futures::Future;
use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

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

pub fn run_cmd_opt(query: &str, command: &str) -> commander::Opt {
    commander::Opt {
        config: None,
        command: commander::Command::Cmd(commander::cmd::Cmd::Run {
            options: CommandOptions {
                raw: false,
                group: false,
                no_progress: false,
                no_std_process_return: true,
            },
            query: query.to_string(),
            command: vec![command.into()],
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

pub fn taskserver_config<P: AsRef<Path>>(
    port: u16,
    with_tls: bool,
    authorized_keys: BTreeMap<String, String>,
    admin_authorized_keys: BTreeMap<String, String>,
    task_server_dir: P,
) -> ServerConfig {
    ServerConfig {
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
        bind_address: format!("127.0.0.1:{}", port),
        data_directory: task_server_dir.as_ref().to_string_lossy().to_string(),
        authorized_keys,
        admin_authorized_keys,
    }
}

pub fn executor_config(
    port: u16,
    with_tls: bool,
    authorized_keys: BTreeMap<String, String>,
) -> ExecutorConfig {
    ExecutorConfig {
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
        client_id: "exec".to_string(),
        tags: Default::default(),
        server_url: format!("http://127.0.0.1:{}", port),
        authorized_keys,
    }
}

pub fn approve_key_executor_cmd() -> commander::Opt {
    commander::Opt {
        config: None,
        command: commander::Command::Admin {
            output_mode: AdminCommandOuputMode::Json,
            command: commander::AdminCommand::ApproveExecutorKey {
                executor: "exec".to_string(),
            },
        },
    }
}
pub fn list_executors_keys_cmd() -> commander::Opt {
    commander::Opt {
        config: None,
        command: commander::Command::Admin {
            output_mode: AdminCommandOuputMode::Json,
            command: commander::AdminCommand::ListExecutorKeys,
        },
    }
}

pub fn commander_config(port: u16, with_tls: bool, ed25519_key: ED25519Key) -> CommanderConfig {
    CommanderConfig {
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
        server_url: format!("http://127.0.0.1:{}", port),
        ed25519_key,
    }
}

pub fn assert_success_of_one_executor(res: CommanderSyntheticOutput) {
    match res {
        CommanderSyntheticOutput::Executor {
            states,
            output: _output,
        } => assert_eq!(
            1,
            states
                .get(&ExecutorState::Success)
                .expect("Executor must be in success")
                .len()
        ),
        _ => panic!("Not an executor result"),
    }
}

pub fn assert_executor_error(res: CommanderSyntheticOutput) {
    match res {
        CommanderSyntheticOutput::Executor {
            states,
            output: _output,
        } => assert_eq!(
            1,
            states
                .get(&ExecutorState::Error)
                .expect("Executor must be in error")
                .len()
        ),
        _ => panic!("Not an executor result"),
    }
}

pub async fn loop_executor_main(
    mut config: ExecutorConfig,
    signing_key: ED25519Key,
) -> Result<(), Box<dyn Error>> {
    loop {
        config = executor_main(config, signing_key.clone()).await?;
    }
}
