#[macro_use]
extern crate log;

use funtonic::config::ServerConfig;
use funtonic::file_utils::mkdirs;
use funtonic::task_server::TaskServer;
use funtonic::tonic;
use grpc_service::grpc_protocol::commander_service_server::CommanderServiceServer;
use grpc_service::grpc_protocol::executor_service_server::ExecutorServiceServer;
use std::path::PathBuf;
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

pub async fn taskserver_main(server_config: ServerConfig) -> anyhow::Result<()> {
    info!(
        "Taskserver v{}, core v{},  protocol v{}, query parser v{} starting",
        VERSION,
        funtonic::VERSION,
        funtonic::PROTOCOL_VERSION,
        funtonic::QUERY_PARSER_VERSION
    );

    info!("{:#?}", server_config);
    let mut server = Server::builder().tcp_keepalive(Some(Duration::from_secs(25)));
    if let Some(tls_config) = &server_config.tls {
        server = server.tls_config(tls_config.get_server_config()?)?;
    }

    let addr = server_config.bind_address.parse().unwrap();
    let database_directory = mkdirs(&server_config.data_directory)?;
    let task_server = TaskServer::new(
        &database_directory,
        &server_config.authorized_keys,
        &server_config.admin_authorized_keys,
    )?;

    task_server.start_heartbeat();

    server
        .add_service(ExecutorServiceServer::new(task_server.clone()))
        .add_service(CommanderServiceServer::new(task_server))
        .serve(addr)
        .await?;

    Ok(())
}
