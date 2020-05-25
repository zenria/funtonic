use executor::{executor_main, Opt};
use funtonic::config;
use funtonic::config::ExecutorConfig;
use funtonic::crypto::keygen::generate_base64_encoded_keys;
use log::{error, info, warn};
use std::fs::File;
use std::path::{Path, PathBuf};
use structopt::StructOpt;
use tokio::time::Duration;

const LOG4RS_CONFIG: &'static str = "/etc/funtonic/executor-log4rs.yaml";

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    log4rs_gelf::init_file(LOG4RS_CONFIG, None).unwrap_or_else(|e| {
        eprintln!("Cannot initialize logger from {} - {}", LOG4RS_CONFIG, e);
        eprintln!("Trying with dev assets!");
        log4rs_gelf::init_file("executor/assets/log4rs.yaml", None)
            .expect("Cannot open executor/assets/log4rs.yaml");
    });
    let opt = Opt::from_args();
    loop {
        let (config, config_path) =
            config::parse::<_, _, ExecutorConfig>(&opt.config, "executor.yml")?;
        let key_path = get_key_path(config::get_config_directory(&opt.config, "executor.yml")?);
        let signing_key = if key_path.exists() {
            serde_yaml::from_reader(File::open(key_path)?)?
        } else {
            let (signing_key, _) = generate_base64_encoded_keys(&config.client_id);
            warn!(
                "Signing key not found, generated a new one, public_key: {}",
                signing_key.public_key.as_ref().unwrap()
            );
            serde_yaml::to_writer(File::create(key_path)?, &signing_key)?;
            signing_key
        };
        match executor_main(config, signing_key).await {
            Err(e) => {
                // this should only happen on TLS configuration parsing.
                error!("Unknown error occured! {}", e);
                tokio::time::delay_for(Duration::from_secs(1)).await;
            }
            Ok(config) => {
                info!("Connection to task server ended gracefully, saving config & reconnecting.");
                if let Err(e) = serde_yaml::to_writer(File::create(&config_path)?, &config) {
                    error!(
                        "Unable to write configuration file to {}: {}\n{:#?}",
                        config_path.to_string_lossy(),
                        e,
                        config
                    );
                }
            }
        }
    }
}

fn get_key_path<P: AsRef<Path>>(config_dir: P) -> PathBuf {
    let mut ret = PathBuf::from(config_dir.as_ref());
    ret.push("executor_ed25519_key.yml");
    ret
}
