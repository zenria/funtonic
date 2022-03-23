#[macro_use]
extern crate log;

use exec::a_sync;
use exec::*;
use funtonic::config::{ED25519Key, ExecutorConfig};
use funtonic::crypto::keystore::{memory_keystore, KeyStore, KeyStoreBackend};
use funtonic::crypto::signed_payload::encode_and_sign;
use funtonic::executor_meta::{ExecutorMeta, Tag};
use funtonic::tokio;
use funtonic::tonic;
use funtonic::PROTOCOL_VERSION;
use futures::StreamExt;
use grpc_service::grpc_protocol::executor_service_client::ExecutorServiceClient;
use grpc_service::grpc_protocol::launch_task_request_payload::Task;
use grpc_service::grpc_protocol::task_execution_result::ExecutionResult;
use grpc_service::grpc_protocol::task_output::Output;
use grpc_service::grpc_protocol::{
    Empty, ExecuteCommand, GetTasksRequest, LaunchTaskRequestPayload, RegisterExecutorRequest,
    TaskCompleted, TaskExecutionResult, TaskOutput,
};
use http::Uri;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::error::Error;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tokio::sync::watch::Sender;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tonic::metadata::AsciiMetadataValue;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(StructOpt, Debug)]
#[structopt(name = "Funtonic executor")]
pub struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    pub config: Option<PathBuf>,
}

#[derive(Error, Debug)]
#[error("Missing field for server config!")]
struct InvalidConfig;

#[derive(Debug, Clone, Copy)]
enum LastConnectionStatus {
    Connecting,
    Connected,
}
/// Launch the executor ; returns an updated version of its configuration. The caller should persist it
/// and immediately reconnect the executor
pub async fn executor_main(
    mut executor_config: ExecutorConfig,
    mut signing_key: ED25519Key,
) -> anyhow::Result<ExecutorConfig> {
    info!(
        "Executor v{}, core v{},  protocol v{}",
        VERSION,
        funtonic::VERSION,
        PROTOCOL_VERSION
    );
    info!("{:#?}", executor_config);

    // force the is of the key to match the executor client_id
    signing_key.id = executor_config.client_id.clone();
    let mut endpoint = Channel::builder(Uri::from_str(&executor_config.server_url)?)
        .tcp_keepalive(Some(Duration::from_secs(60)));

    if let Some(tls_config) = &executor_config.tls {
        endpoint = endpoint.tls_config(tls_config.get_client_config()?)?;
    }

    let max_reconnect_time = Duration::from_secs(10);
    let mut reconnect_time = Duration::from_millis(100);

    let key_store = memory_keystore().init_from_map(&executor_config.authorized_keys)?;

    let mut executor_meta = ExecutorMeta::from(&executor_config);
    // add some generic meta about system
    let info = os_info::get();
    let mut os: HashMap<String, Tag> = HashMap::new();
    os.insert("type".into(), format!("{:?}", info.os_type()).into());
    os.insert("version".into(), format!("{}", info.version()).into());
    executor_meta.tags_mut().insert("os".into(), Tag::Map(os));
    info!("Metas: {:#?}", executor_meta);

    let (mut connection_status_sender, connection_status_receiver) =
        tokio::sync::watch::channel(LastConnectionStatus::Connecting);

    // executor execution never ends
    'retryloop: loop {
        match do_executor_main(
            &endpoint,
            &executor_meta,
            &executor_config,
            &mut connection_status_sender,
            &key_store,
            signing_key.clone(),
        )
        .await
        {
            Ok(config_modification) => {
                match config_modification {
                    ConfigurationModification::AddKey { key_id, key_bytes } => {
                        executor_config
                            .authorized_keys
                            .insert(key_id, base64::encode(key_bytes));
                    }
                    ConfigurationModification::RevokeKey(key_id) => {
                        executor_config.authorized_keys.remove(&key_id);
                    }

                    ConfigurationModification::None => {
                        // nothing to do here
                    }
                }
                break 'retryloop;
            }
            Err(e) => {
                error!("Error {}", e);
                // increase reconnect time if connecting, reset if connected
                match *connection_status_receiver.borrow() {
                    LastConnectionStatus::Connecting => {
                        reconnect_time = reconnect_time + Duration::from_secs(1);
                        if reconnect_time > max_reconnect_time {
                            reconnect_time = max_reconnect_time;
                        }
                    }
                    LastConnectionStatus::Connected => reconnect_time = Duration::from_secs(1),
                }
                info!("Reconnecting in {}s", reconnect_time.as_secs());
                tokio::time::sleep(reconnect_time).await;
            }
        }
    }

    Ok(executor_config)
}

enum ConfigurationModification {
    AddKey { key_id: String, key_bytes: Vec<u8> },
    RevokeKey(String),
    None,
}

async fn do_executor_main<B: KeyStoreBackend>(
    endpoint: &Endpoint,
    executor_metas: &ExecutorMeta,
    executor_config: &ExecutorConfig,
    last_connection_status_sender: &mut Sender<LastConnectionStatus>,
    key_store: &KeyStore<B>,
    signing_key: ED25519Key,
) -> anyhow::Result<ConfigurationModification> {
    last_connection_status_sender.send(LastConnectionStatus::Connecting)?;
    let channel = endpoint.connect().await?;
    last_connection_status_sender.send(LastConnectionStatus::Connected)?;

    let mut client = ExecutorServiceClient::new(channel);

    info!("Connected");

    let client_id = executor_metas.client_id().to_string();

    let request = tonic::Request::new(
        RegisterExecutorRequest {
            public_key: base64::decode(signing_key.public_key.as_ref().unwrap())?,
            client_id: client_id.clone(),
            get_tasks_request: Some(encode_and_sign(
                GetTasksRequest::try_from(executor_config)?,
                &signing_key,
                Duration::from_secs(60),
            )?),
        }
        .into(),
    );

    let mut response = client.get_tasks(request).await?.into_inner();

    while let Some(task) = response.message().await? {
        // by convention this field is always here, so we can "safely" unwrap
        let task_id = task.task_id;

        let task_payload = task.payload;
        match task_payload {
            Some(signed_payload) => {
                match key_store.decode_payload::<LaunchTaskRequestPayload>(&signed_payload) {
                    Ok(task) => match task.task {
                        Some(task) => match task {
                            Task::ExecuteCommand(cmd) => {
                                info!("Received task {} - {}", task_id, cmd.command);
                                tokio::spawn(execute_task(
                                    cmd,
                                    task_id,
                                    client_id.clone(),
                                    client.clone(),
                                    signing_key.clone(),
                                ));
                            }
                            Task::StreamingPayload(_) => {
                                error!("Streaming not yet implemented!");
                                // reject task
                                single_execution_result(
                                    ExecutionResult::TaskRejected(
                                        "Streaming not yet implemented".into(),
                                    ),
                                    &client_id,
                                    &task_id,
                                    &signing_key,
                                    &mut client,
                                )
                                .await?;
                            }
                            Task::AuthorizeKey(public_key) => {
                                single_execution_result(
                                    ExecutionResult::TaskCompleted(TaskCompleted {
                                        return_code: 0,
                                    }),
                                    &client_id,
                                    &task_id,
                                    &signing_key,
                                    &mut client,
                                )
                                .await?;
                                return Ok(ConfigurationModification::AddKey {
                                    key_id: public_key.key_id,
                                    key_bytes: public_key.key_bytes,
                                });
                            }
                            Task::RevokeKey(key_id) => {
                                if executor_config.authorized_keys.contains_key(&key_id) {
                                    single_execution_result(
                                        ExecutionResult::TaskCompleted(TaskCompleted {
                                            return_code: 0,
                                        }),
                                        &client_id,
                                        &task_id,
                                        &signing_key,
                                        &mut client,
                                    )
                                    .await?;
                                    return Ok(ConfigurationModification::RevokeKey(key_id));
                                } else {
                                    single_execution_result(
                                        ExecutionResult::TaskRejected(format!(
                                            "Cannot revoke key {}: key not found",
                                            key_id
                                        )),
                                        &client_id,
                                        &task_id,
                                        &signing_key,
                                        &mut client,
                                    )
                                    .await?;
                                }
                            }
                        },
                        None => error!("No task inside LauchTaskRequest"),
                    },
                    Err(e) => {
                        error!("Unable to decode received payload for {}: {}", task_id, e);
                        // reject task
                        single_execution_result(
                            ExecutionResult::TaskRejected(format!(
                                "Unable to decode received payload for {}: {}",
                                task_id, e
                            )),
                            &client_id,
                            &task_id,
                            &signing_key,
                            &mut client,
                        )
                        .await?;
                    }
                }
            }
            None => error!("No payload in {}, ignoring it!", task_id),
        }

        info!("Waiting for next task")
    }

    Ok(ConfigurationModification::None)
}

async fn single_execution_result(
    result: ExecutionResult,
    client_id: &str,
    task_id: &str,
    signing_key: &ED25519Key,
    client: &mut ExecutorServiceClient<Channel>,
) -> anyhow::Result<()> {
    let stream = futures::stream::once(futures::future::ready(encode_and_sign(
        TaskExecutionResult {
            task_id: task_id.to_string(),
            client_id: client_id.to_string(),
            execution_result: Some(result),
        },
        &signing_key,
        Duration::from_secs(60),
    )?));
    let mut request = Request::new(stream);
    request.metadata_mut().insert(
        "task_id",
        AsciiMetadataValue::from_str(&task_id.clone()).unwrap(),
    );
    client.task_execution(request).await?;
    Ok(())
}

async fn execute_task(
    task_payload: ExecuteCommand,
    task_id: String,
    client_id: String,
    client: ExecutorServiceClient<Channel>,
    signing_key: ED25519Key,
) {
    match do_execute_task(task_payload, task_id, client_id, client, signing_key).await {
        Ok(_) => (),
        Err(e) => error!("Something wrong happened while executing task {}", e),
    }
}

async fn do_execute_task(
    execute_command: ExecuteCommand,
    task_id: String,
    client_id: String,
    mut client: ExecutorServiceClient<Channel>,
    signing_key: ED25519Key,
) -> Result<(), Box<dyn Error>> {
    let cloned_task_id = task_id.clone();
    let cloned_client_id = client_id.clone();

    let (exec_receiver, kill_sender) = a_sync::exec_command(&execute_command.command)?;

    let stream = UnboundedReceiverStream::new(exec_receiver)
        .map(|exec_event| match exec_event {
            ExecEvent::Started => ExecutionResult::Ping(Empty {}),
            ExecEvent::Finished(return_code) => match return_code {
                None => ExecutionResult::TaskAborted(Empty {}),
                Some(return_code) => ExecutionResult::TaskCompleted(TaskCompleted { return_code }),
            },
            ExecEvent::LineEmitted(line) => ExecutionResult::TaskOutput(TaskOutput {
                output: Some(match &line.line_type {
                    Type::Out => Output::Stdout(line.line),
                    Type::Err => Output::Stderr(line.line),
                }),
            }),
        })
        .map(move |execution_result| TaskExecutionResult {
            task_id: task_id.clone(),
            client_id: cloned_client_id.clone(),
            execution_result: Some(execution_result),
        })
        .map(move |execution_result| {
            encode_and_sign(execution_result, &signing_key, Duration::from_secs(60))
        })
        .filter(|result| match result {
            // filter out signing error
            Ok(_) => futures::future::ready(true),
            Err(e) => {
                error!("Unable to sign task execution result {}", e);
                futures::future::ready(false)
            }
        })
        .map(|result| result.unwrap());

    let mut request = Request::new(stream);
    request.metadata_mut().insert(
        "task_id",
        AsciiMetadataValue::from_str(&cloned_task_id).unwrap(),
    );
    client.task_execution(request).await?;
    // do not leave process behind
    let _ = kill_sender.send(());
    info!("Finished task {}", cloned_task_id);
    Ok(())
}
