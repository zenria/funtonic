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

const LOG4RS_CONFIG: &'static str = "/etc/funtonic/server-log4rs.yaml";

#[derive(StructOpt, Debug)]
#[structopt(name = "Funtonic taskserver")]
struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    config: Option<PathBuf>,
}

#[derive(Error, Debug)]
#[error("Missing field for server config!")]
struct InvalidConfig;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log4rs_gelf::init_file(LOG4RS_CONFIG, None).unwrap_or_else(|e| {
        eprintln!("Cannot initialize logger from {} - {}", LOG4RS_CONFIG, e);
        eprintln!("Trying with dev assets!");
        log4rs_gelf::init_file("executor/assets/log4rs.yaml", None)
            .expect("Cannot open executor/assets/log4rs.yaml");
    });

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
