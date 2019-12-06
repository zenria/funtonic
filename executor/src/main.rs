use executor::{executor_main, Opt};
use funtonic::config::Config;
use structopt::StructOpt;

const LOG4RS_CONFIG: &'static str = "/etc/funtonic/executor-log4rs.yaml";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log4rs_gelf::init_file(LOG4RS_CONFIG, None).unwrap_or_else(|e| {
        eprintln!("Cannot initialize logger from {} - {}", LOG4RS_CONFIG, e);
        eprintln!("Trying with dev assets!");
        log4rs_gelf::init_file("executor/assets/log4rs.yaml", None)
            .expect("Cannot open executor/assets/log4rs.yaml");
    });
    let opt = Opt::from_args();
    let config = Config::parse(&opt.config, "executor.yml")?;
    executor_main(config).await
}
