use funtonic::config::{Config};
use structopt::StructOpt;
use commander::{Opt, commander_main};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opt: Opt = Opt::from_args();
    let config = Config::parse(&opt.config, "commander.yml")?;
    commander_main(opt, config).await
}
