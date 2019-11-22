#[macro_use]
extern crate log;

use funtonic::config::{Config, Role};
use funtonic::file_utils::{mkdirs, path_concat2};
use funtonic::task_server::TaskServer;
use grpc_service::server::TasksManagerServer;
use std::path::{Path, PathBuf};
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tonic::transport::Server;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[derive(StructOpt, Debug)]
#[structopt(name = "basic")]
struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    config: Option<PathBuf>,
}

#[derive(Error, Debug)]
#[error("Missing field for server config!")]
struct InvalidConfig;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .without_time()
            .finish(),
    )
    .expect("setting tracing default failed");
    tracing_log::LogTracer::init().unwrap();

    let opt = Opt::from_args();
    let config = Config::parse(&opt.config, "server.yml")?;
    info!("Server starting with config {:#?}", config);
    if let Role::Server(server_config) = &config.role {
        let mut server = Server::builder().tcp_keepalive(Some(Duration::from_secs(25)));
        if let Some(tls_config) = &config.tls {
            server = server.tls_config(tls_config.get_server_config()?);
        }

        let addr = server_config.bind_address.parse().unwrap();
        let database_directory = mkdirs(&server_config.data_directory)?;
        let database_path = path_concat2(database_directory, "known_executors.yml");
        let task_server = TaskServer::new(
            Path::new(&database_path),
            server_config.authorized_client_tokens.clone(),
        )?;

        server
            .add_service(TasksManagerServer::new(task_server))
            .serve(addr)
            .await?;

        Ok(())
    } else {
        Err(InvalidConfig)?
    }
}