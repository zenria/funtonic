#[macro_use]
extern crate log;

pub use crate::admin::{AdminCommand, AdminCommandOuputMode};
use anyhow::Context;
use colored::{Color, Colorize};
use funtonic::config::{CommanderConfig, ED25519Key};
use funtonic::crypto::keygen::generate_ed25519_key_pair;
use funtonic::{data_encoding, tonic};
use grpc_service::grpc_protocol::commander_service_client::CommanderServiceClient;
use http::Uri;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Error, Formatter};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tonic::transport::Channel;

mod admin;
pub mod cmd;

#[derive(Eq, Ord, PartialOrd, PartialEq, Hash, Debug)]
pub enum ExecutorState {
    Matching,
    Submitted,
    Alive,
    Disconnected,
    Error,
    Success,
}
impl Display for ExecutorState {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        match self {
            ExecutorState::Matching => write!(f, "{}", "Matching".color(self.color())),
            ExecutorState::Submitted => write!(f, "{}", "Submitted".color(self.color())),
            ExecutorState::Alive => write!(f, "{}", "Alive".color(self.color())),
            ExecutorState::Disconnected => write!(f, "{}", "Disconnected".color(self.color())),
            ExecutorState::Error => write!(f, "{}", "Error".color(self.color())),
            ExecutorState::Success => write!(f, "{}", "Success".color(self.color())),
        }
    }
}
impl ExecutorState {
    fn color(&self) -> Color {
        match self {
            ExecutorState::Matching => Color::BrightWhite,
            ExecutorState::Submitted => Color::Yellow,
            ExecutorState::Alive => Color::Yellow,
            ExecutorState::Disconnected => Color::Red,
            ExecutorState::Error => Color::Red,
            ExecutorState::Success => Color::Green,
        }
    }
}

#[derive(StructOpt, Debug)]
#[structopt(name = "Funtonic commander")]
pub struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    pub config: Option<PathBuf>,
    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(StructOpt, Debug)]
pub enum Command {
    /// Admin commands
    Admin {
        /// json (js), pretty-json (pjs) or human-readable (hr) or human-readable-long (hrl)
        #[structopt(short = "o", long = "output-mode", default_value = "human-readable")]
        output_mode: AdminCommandOuputMode,
        #[structopt(subcommand)]
        command: AdminCommand,
    },
    #[structopt(flatten)]
    Cmd(cmd::Cmd),
    /// Utilities
    #[structopt(name = "utils")]
    Utils(Utils),
}

#[derive(StructOpt, Debug)]
pub enum Utils {
    /// Generate an ED25519 key pair to be used in various configuration places
    #[structopt(name = "genkey")]
    GenerateED25519KeyPair {
        /// name of the key.
        name: String,
    },
}

#[derive(Error, Debug)]
#[error("Missing field for commander config!")]
struct InvalidConfig;

pub async fn commander_main(
    opt: Opt,
    commander_config: CommanderConfig,
) -> Result<CommanderSyntheticOutput, Box<dyn std::error::Error>> {
    debug!("Commander starting with config {:#?}", commander_config);
    let mut channel = Channel::builder(Uri::from_str(&commander_config.server_url)?)
        .tcp_keepalive(Some(Duration::from_secs(60)));
    if let Some(tls_config) = &commander_config.tls {
        info!("TLS configuration found");
        channel = channel.tls_config(tls_config.get_client_config()?)?;
    }
    let channel = channel
        .connect()
        .await
        .context("Unable to connect to taskserver")?;

    let client = CommanderServiceClient::new(channel);

    info!("Connected");

    match opt.command {
        Command::Admin {
            output_mode,
            command,
        } => admin::handle_admin_command(client, &commander_config, command, output_mode).await,
        Command::Cmd(cmd) => cmd::handle_cmd(client, &commander_config, cmd).await,

        Command::Utils(cmd) => handle_utils_cmd(cmd),
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct GenerateKeyPairOutput {
    ed25519_key: ED25519Key,
    authorized_keys: BTreeMap<String, String>,
}

fn handle_utils_cmd(cmd: Utils) -> Result<CommanderSyntheticOutput, Box<dyn std::error::Error>> {
    match cmd {
        Utils::GenerateED25519KeyPair { name } => {
            let (priv_key, pub_key) = generate_ed25519_key_pair().unwrap();
            let out = GenerateKeyPairOutput {
                ed25519_key: ED25519Key {
                    id: name.clone(),
                    pkcs8: data_encoding::BASE64.encode(&priv_key),
                    public_key: Some(data_encoding::BASE64.encode(&pub_key)),
                },
                authorized_keys: vec![(name, data_encoding::BASE64.encode(&pub_key))]
                    .into_iter()
                    .collect(),
            };
            println!("Generated Keys:\n{}", serde_yaml::to_string(&out)?);
        }
    }
    Ok(CommanderSyntheticOutput::Cmd)
}
#[derive(Debug)]
pub enum CommanderSyntheticOutput {
    Executor {
        states: BTreeMap<ExecutorState, BTreeSet<String>>,
        output: HashMap<String, Vec<String>>,
    },
    Admin(String),
    Cmd,
}
