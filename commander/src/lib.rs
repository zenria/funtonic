#[macro_use]
extern crate log;

use crate::admin::{AdminCommand, AdminCommandOuputMode};
use colored::{Color, Colorize};
use funtonic::config::{Config, Role};
use grpc_service::grpc_protocol::tasks_manager_client::TasksManagerClient;
use http::Uri;
use std::fmt::{Display, Error, Formatter};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tonic::transport::Channel;

mod admin;
pub mod cmd;

#[derive(Eq, Ord, PartialOrd, PartialEq, Hash)]
enum ExecutorState {
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
    /// Run command line programs
    #[structopt(name = "run")]
    Run(cmd::Cmd),
}

#[derive(Error, Debug)]
#[error("Missing field for commander config!")]
struct InvalidConfig;

pub async fn commander_main(opt: Opt, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    debug!("Commander starting with config {:#?}", config);
    if let Role::Commander(commander_config) = &config.role {
        let mut channel = Channel::builder(Uri::from_str(&commander_config.server_url)?)
            .tcp_keepalive(Some(Duration::from_secs(60)));
        if let Some(tls_config) = &config.tls {
            channel = channel.tls_config(tls_config.get_client_config()?);
        }
        let channel = channel.connect().await?;

        let client = TasksManagerClient::new(channel);

        match opt.command {
            Command::Admin {
                output_mode,
                command,
            } => admin::handle_admin_command(client, commander_config, command, output_mode).await,
            Command::Run(cmd) => cmd::handle_cmd(client, commander_config, cmd).await,
        }
    } else {
        Err(InvalidConfig)?
    }
}
