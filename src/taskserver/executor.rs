use funtonic::generated::tasks::client::TasksManagerClient;
use funtonic::generated::tasks::server::TasksManager;
use funtonic::generated::tasks::task_execution_stream::ExecutionResult;
use funtonic::generated::tasks::task_output::Output;
use funtonic::generated::tasks::{
    GetTasksRequest, LaunchTaskRequest, TaskCompleted, TaskExecutionStream, TaskOutput, TaskPayload,
};
use tonic::Request;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = TasksManagerClient::connect("http://[::1]:50051")?;

    let client_id = std::env::args().nth(1).unwrap();

    let request = tonic::Request::new(GetTasksRequest {
        client_id: client_id.to_string(),
    });

    let mut response = client.get_tasks(request).await?.into_inner();

    while let Some(task) = response.message().await? {
        // by convention this field is always here, so we can "safely" unwrap
        let task_id = task.task_id;
        let task_payload = task.task_payload.unwrap();
        println!("Recieved task {} - {}", task_id, task_payload.payload);
        // TODO EXECUTE PAYLOAD
        let stream = futures_util::stream::iter(vec![
            TaskExecutionStream {
                task_id: task_id.clone(),
                client_id: client_id.to_string(),
                execution_result: Some(ExecutionResult::TaskOutput(TaskOutput {
                    output: Some(Output::Stdout("coucou les amis\n".to_string())),
                })),
            },
            TaskExecutionStream {
                task_id: task_id.clone(),
                client_id: client_id.to_string(),
                execution_result: Some(ExecutionResult::TaskOutput(TaskOutput {
                    output: Some(Output::Stdout("recoucou\n".to_string())),
                })),
            },
            TaskExecutionStream {
                task_id: task_id.clone(),
                client_id: client_id.to_string(),
                execution_result: Some(ExecutionResult::TaskCompleted(TaskCompleted {
                    return_code: 123,
                })),
            },
        ]);

        client.task_execution(Request::new(stream)).await?;
        println!("Waiting for next task")
    }

    Ok(())
}
