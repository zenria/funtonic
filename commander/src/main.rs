use clap::Parser;
use commander::{commander_main, Opt};
use funtonic::config;
use funtonic::tokio;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::FmtSubscriber;

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
    let opt: Opt = Opt::parse();
    let (config, _) = config::parse(&opt.config, "commander.yml")?;
    commander_main(opt, config).await.map(|_| ())
}
