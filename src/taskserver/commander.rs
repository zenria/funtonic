#[macro_use]
extern crate log;

use funtonic::config::{Config, Role};
use funtonic::generated::tasks::client::TasksManagerClient;
use funtonic::generated::tasks::task_execution_result::ExecutionResult;
use funtonic::generated::tasks::task_output::Output;
use funtonic::generated::tasks::{LaunchTaskRequest, TaskPayload};
use http::Uri;
use std::path::PathBuf;
use std::str::FromStr;
use structopt::StructOpt;
use thiserror::Error;
use tonic::transport::Channel;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

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
    let config = Config::parse(&opt.config, "commander.yml")?;
    debug!("Commander starting with config {:#?}", config);
    if let Role::Commander(commander_config) = &config.role {
        let mut channel = Channel::builder(Uri::from_str(&commander_config.server_url)?);
        if let Some(tls_config) = &config.tls {
            channel.tls_config(&tls_config.get_client_config()?);
        }
        let channel = channel.channel();

        let mut client = TasksManagerClient::new(channel);

        let command = opt.command.join(" ");

        let request = tonic::Request::new(LaunchTaskRequest {
            task_payload: Some(TaskPayload { payload: command }),
            predicate: "*".to_string(),
        });

        let mut response = client.launch_task(request).await?.into_inner();

        while let Some(task_execution_result) = response.message().await? {
            // by convention this field is always here, so we can "safely" unwrap
            let execution_result = task_execution_result.execution_result.as_ref().unwrap();
            let client_id = &task_execution_result.client_id;
            match execution_result {
                ExecutionResult::TaskCompleted(completion) => {
                    println!(
                        "Tasks completed on {} with exit code: {}",
                        client_id, completion.return_code
                    );
                }
                ExecutionResult::TaskOutput(output) => {
                    if let Some(output) = output.output.as_ref() {
                        match output {
                            Output::Stdout(o) => print!("{}: {}", client_id, o),
                            Output::Stderr(e) => eprint!("{}: {}", client_id, e),
                        }
                    }
                }
                ExecutionResult::Ping(_) => {
                    debug!("Pinged!");
                }
                ExecutionResult::Disconnected(_) => {
                    error!("{} disconnected!", client_id);
                    eprintln!("{} disconnected!", client_id);
                }
            }
        }

        Ok(())
    } else {
        Err(InvalidConfig)?
    }
}
