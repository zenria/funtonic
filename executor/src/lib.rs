#[macro_use]
extern crate log;

use exec::a_sync;
use exec::*;
use funtonic::config::{Config, Role};
use funtonic::executor_meta::{ExecutorMeta, Tag};
use funtonic::PROTOCOL_VERSION;
use futures::StreamExt;
use grpc_service::task_execution_result::ExecutionResult;
use grpc_service::task_output::Output;
use grpc_service::tasks_manager_client::TasksManagerClient;
use grpc_service::{Empty, TaskCompleted, TaskExecutionResult, TaskOutput};
use http::Uri;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tokio::sync::watch::Sender;
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

pub async fn executor_main(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        "Executor v{}, core v{},  protocol v{}",
        VERSION,
        funtonic::VERSION,
        PROTOCOL_VERSION
    );
    info!("{:#?}", config);

    if let Role::Executor(executor_config) = config.role {
        let mut endpoint = Channel::builder(Uri::from_str(&executor_config.server_url)?)
            .tcp_keepalive(Some(Duration::from_secs(60)));

        if let Some(tls_config) = config.tls {
            endpoint = endpoint.tls_config(tls_config.get_client_config()?);
        }

        let max_reconnect_time = Duration::from_secs(10);
        let mut reconnect_time = Duration::from_millis(100);

        let mut executor_meta = ExecutorMeta::from(executor_config);
        // add some generic meta about system
        let info = os_info::get();
        let mut os: HashMap<String, Tag> = HashMap::new();
        os.insert("type".into(), format!("{:?}", info.os_type()).into());
        os.insert("version".into(), format!("{}", info.version()).into());
        executor_meta.tags_mut().insert("os".into(), Tag::Map(os));
        info!("Metas: {:#?}", executor_meta);

        let (mut connection_status_sender, connection_status_receiver) =
            tokio::sync::watch::channel(LastConnectionStatus::Connecting);

        while let Err(e) =
            do_executor_main(&endpoint, &executor_meta, &mut connection_status_sender).await
        {
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
            tokio::time::delay_for(reconnect_time).await;
        }
        Ok(())
    } else {
        Err(InvalidConfig)?
    }
}

async fn do_executor_main(
    endpoint: &Endpoint,
    executor_metas: &ExecutorMeta,
    last_connection_status_sender: &mut Sender<LastConnectionStatus>,
) -> Result<(), Box<dyn std::error::Error>> {
    last_connection_status_sender.broadcast(LastConnectionStatus::Connecting)?;
    let channel = endpoint.connect().await?;
    last_connection_status_sender.broadcast(LastConnectionStatus::Connected)?;

    let mut client = TasksManagerClient::new(channel);

    info!("Connected");

    let client_id = executor_metas.client_id().to_string();

    let request = tonic::Request::new(executor_metas.into());

    let mut response = client.get_tasks(request).await?.into_inner();

    while let Some(task) = response.message().await? {
        // by convention this field is always here, so we can "safely" unwrap
        let task_id = task.task_id;
        let task_payload = task.task_payload.unwrap();
        info!("Received task {} - {}", task_id, task_payload.payload);

        let cloned_task_id = task_id.clone();
        let cloned_client_id = client_id.clone();

        let (exec_receiver, kill_sender) = a_sync::exec_command(&task_payload.payload)?;

        let stream = exec_receiver
            .map(|exec_event| match exec_event {
                ExecEvent::Started => ExecutionResult::Ping(Empty {}),
                ExecEvent::Finished(return_code) => {
                    ExecutionResult::TaskCompleted(TaskCompleted { return_code })
                }
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
            });

        let mut request = Request::new(stream);
        request.metadata_mut().insert(
            "task_id",
            AsciiMetadataValue::from_str(&cloned_task_id).unwrap(),
        );
        client.task_execution(request).await?;
        // do not leave process behind
        let _ = kill_sender.send(());
        info!("Waiting for next task")
    }

    Ok(())
}
