use funtonic::generated::tasks::client::TasksManagerClient;
use funtonic::generated::tasks::server::TasksManager;
use funtonic::generated::tasks::task_execution_result::ExecutionResult;
use funtonic::generated::tasks::task_output::Output;
use funtonic::generated::tasks::{LaunchTaskRequest, TaskPayload};
use itertools::Itertools;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = TasksManagerClient::connect("http://[::1]:50051")?;

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
                println!("Pinged!");
            }
        }
    }

    Ok(())
}
