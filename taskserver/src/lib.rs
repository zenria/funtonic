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

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(StructOpt, Debug)]
#[structopt(name = "Funtonic taskserver")]
pub struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    pub config: Option<PathBuf>,
}

#[derive(Error, Debug)]
#[error("Missing field for server config!")]
struct InvalidConfig;

pub async fn taskserver_main(config: Config) -> Result<(), anyhow::Error> {
    info!(
        "Taskserver v{}, core v{},  protocol v{}",
        VERSION,
        funtonic::VERSION,
        funtonic::PROTOCOL_VERSION
    );

    info!("{:#?}", config);
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