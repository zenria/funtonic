#[macro_use]
extern crate log;

use itertools::Itertools;

use funtonic::generated::tasks::client::TasksManagerClient;
use funtonic::generated::tasks::task_execution_result::ExecutionResult;
use funtonic::generated::tasks::task_output::Output;
use funtonic::generated::tasks::{LaunchTaskRequest, TaskPayload};
use tracing_subscriber::{EnvFilter, FmtSubscriber};
use tonic::transport::{Certificate, ClientTlsConfig, Channel, Identity};

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

    let pem = tokio::fs::read("tls/funtonic-ca.pem").await?;
    let ca = Certificate::from_pem(pem);

    let cert = tokio::fs::read("tls/commander.pem").await?;
    let key = tokio::fs::read("tls/commander-key.pem").await?;
    let identity = Identity::from_pem(cert, key);


    let tls = ClientTlsConfig::with_rustls()
        .ca_certificate(ca)
        .identity(identity)
        .domain_name("server.example.funtonic")
        .clone();

    let channel = Channel::from_static("http://[::1]:50051")
        .tls_config(&tls)
        .channel();


    let mut client = TasksManagerClient::new(channel);

    let command = std::env::args().skip(1).join(" ");

    if command.len() == 0 {
        eprintln!("Please specify a command to run");
        std::process::exit(1);
    }

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
        }
    }

    Ok(())
}
