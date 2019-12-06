use commander::{commander_main, Opt};
use funtonic::config::Config;
use structopt::StructOpt;
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
    let opt: Opt = Opt::from_args();
    let config = Config::parse(&opt.config, "commander.yml")?;
    commander_main(opt, config).await
}
