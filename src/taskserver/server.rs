#[macro_use]
extern crate log;

use funtonic::config::{Config, Role};
use funtonic::file_utils::read;
use funtonic::generated::tasks::server::TasksManagerServer;
use funtonic::task_server::TaskServer;
use std::path::PathBuf;
use structopt::StructOpt;
use thiserror::Error;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
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
        let mut server = Server::builder();
        if let Some(tls_config) = &config.tls {
            let cert = read(&tls_config.cert)?;
            let key = read(&tls_config.key)?;
            let identity = Identity::from_pem(cert, key);
            let ca = read(&tls_config.ca_cert)?;
            let ca = Certificate::from_pem(ca);
            server.tls_config(
                ServerTlsConfig::with_rustls()
                    .identity(identity)
                    .client_ca_root(ca),
            );
        }

        let addr = server_config.bind_address.parse().unwrap();
        let task_server = TaskServer::new();

        server
            .serve(addr, TasksManagerServer::new(task_server))
            .await?;

        Ok(())
    } else {
        Err(InvalidConfig)?
    }
}
