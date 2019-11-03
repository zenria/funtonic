#[macro_use]
extern crate log;

use funtonic::config::{Config, Role};
use funtonic::exec::Type::Out;
use funtonic::exec::*;
use funtonic::generated::tasks::client::TasksManagerClient;
use funtonic::generated::tasks::task_execution_result::ExecutionResult;
use funtonic::generated::tasks::task_output::Output;
use funtonic::generated::tasks::{
    GetTasksRequest, TaskAlive, TaskCompleted, TaskExecutionResult, TaskOutput,
};
use futures_util::StreamExt;
use http::Uri;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tonic::metadata::AsciiMetadataValue;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tracing_subscriber::{EnvFilter, FmtSubscriber};
use funtonic::VERSION;
use funtonic::executor_meta::ExecutorMeta;

#[derive(StructOpt, Debug)]
#[structopt(name = "basic")]
struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    config: Option<PathBuf>,
    command: Vec<String>,
}

#[derive(Error, Debug)]
#[error("Missing field for server config!")]
struct InvalidConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .without_time()
            .finish(),
    )
    .expect("setting tracing default failed");
    tracing_log::LogTracer::init().unwrap();
    let opt = Opt::from_args();
    let config = Config::parse(&opt.config, "executor.yml")?;
    info!("Executor starting with config {:#?}", config);
    if let Role::Executor(executor_config) = &config.role {
        let mut endpoint = Channel::builder(Uri::from_str(&executor_config.server_url)?);
        if let Some(tls_config) = &config.tls {
            endpoint.tls_config(&tls_config.get_client_config()?);
        }

        let max_reconnect_time = Duration::from_secs(10);
        let mut reconnect_time = Duration::from_secs(1);
        while let Err(e) = executor_main(&endpoint, &executor_config.executor_metas).await {
            error!("Error {}", e);
            info!("Reconnecting in {}s", reconnect_time.as_secs());
            tokio::timer::delay_for(reconnect_time).await;
            // increase reconnect time
            reconnect_time = reconnect_time + Duration::from_secs(1);
            if reconnect_time > max_reconnect_time {
                reconnect_time = max_reconnect_time;
            }
            info!("Reconnecting...")
        }
        Ok(())
    } else {
        Err(InvalidConfig)?
    }
}

async fn executor_main(
    endpoint: &Endpoint,
    executor_metas: &ExecutorMeta,
) -> Result<(), Box<dyn std::error::Error>> {
    let channel = endpoint.channel();

    let mut client = TasksManagerClient::new(channel);

    info!("Connected");

    let client_id = executor_metas.client_id().to_string();

    let request = tonic::Request::new(GetTasksRequest {
        client_id: client_id.clone(),
        client_version: VERSION.into(),
        tags: Default::default()
    });

    let mut response = client.get_tasks(request).await?.into_inner();

    while let Some(task) = response.message().await? {
        // by convention this field is always here, so we can "safely" unwrap
        let task_id = task.task_id;
        let task_payload = task.task_payload.unwrap();
        info!("Received task {} - {}", task_id, task_payload.payload);

        let (mut sender, receiver) = tokio_sync::mpsc::unbounded_channel();
        // unconditionnaly ping so the task will be "consumed" on the server
        if let Err(_) = sender.try_send(ExecutionResult::Ping(TaskAlive {})) {}
        // TODO handle error (this should also be an event)
        let (exec_receiver, kill_sender) = exec_command(&task_payload.payload).unwrap();
        tokio_executor::blocking::run(move || {
            for exec_event in exec_receiver {
                let execution_result = match exec_event {
                    ExecEvent::Started => ExecutionResult::Ping(TaskAlive {}),
                    ExecEvent::Finished(return_code) => {
                        ExecutionResult::TaskCompleted(TaskCompleted { return_code })
                    }
                    ExecEvent::LineEmitted(line) => ExecutionResult::TaskOutput(TaskOutput {
                        output: Some(match &line.line_type {
                            Out => Output::Stdout(String::from_utf8_lossy(&line.line).into()),
                            Type::Err => Output::Stderr(String::from_utf8_lossy(&line.line).into()),
                        }),
                    }),
                };
                // ignore send errors, continue to consume the execution results
                let _ = sender.try_send(execution_result);
            }
        });

        let cloned_task_id = task_id.clone();
        let cloned_client_id = client_id.clone();
        let stream = receiver.map(move |execution_result| TaskExecutionResult {
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
        let _ = kill_sender.try_send(());
        info!("Waiting for next task")
    }

    Ok(())
}
