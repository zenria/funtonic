use funtonic::generated::tasks::server::TasksManagerServer;
use funtonic::task_server::TaskServer;
use tonic::transport::Server;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

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

    let addr = "[::1]:50051".parse().unwrap();
    let task_server = TaskServer::new();

    Server::builder()
        .serve(addr, TasksManagerServer::new(task_server))
        .await?;

    Ok(())
}
