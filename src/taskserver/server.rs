use funtonic::generated::tasks::server::TasksManagerServer;
use funtonic::task_server::TaskServer;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse().unwrap();
    let task_server = TaskServer::new();

    Server::builder()
        .serve(addr, TasksManagerServer::new(task_server))
        .await?;

    Ok(())
}
