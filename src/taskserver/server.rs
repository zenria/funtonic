use funtonic::generated::tasks::server::TasksManagerServer;
use funtonic::task_server::TaskServer;
use tonic::transport::{Server, Identity, ServerTlsConfig, Certificate};
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


    let cert = tokio::fs::read("tls/server.pem").await?;
    let key = tokio::fs::read("tls/server-key.pem").await?;

    let identity = Identity::from_pem(cert, key);

    let ca = tokio::fs::read("tls/funtonic-ca.pem").await?;
    let ca = Certificate::from_pem(ca);


    let addr = "[::1]:50051".parse().unwrap();
    let task_server = TaskServer::new();

    Server::builder()
        .tls_config(ServerTlsConfig::with_rustls().identity(identity).client_ca_root(ca))
        .clone()
        .serve(addr, TasksManagerServer::new(task_server))
        .await?;

    Ok(())
}
