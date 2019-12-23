use funtonic::config::Config;
use structopt::StructOpt;
use taskserver::{taskserver_main, Opt};

const LOG4RS_CONFIG: &'static str = "/etc/funtonic/server-log4rs.yaml";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log4rs_gelf::init_file(LOG4RS_CONFIG, None).unwrap_or_else(|e| {
        eprintln!("Cannot initialize logger from {} - {}", LOG4RS_CONFIG, e);
        eprintln!("Trying with dev assets!");
        log4rs_gelf::init_file("taskserver/assets/log4rs.yaml", None)
            .expect("Cannot open taskserver/assets/log4rs.yaml");
    });
    let opt = Opt::from_args();
    let config = Config::parse(&opt.config, "server.yml")?;
    taskserver_main(config).await
}
