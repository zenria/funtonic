use crate::admin::AdminCommandOuputMode::HumanReadableShort;
use crate::CommanderSyntheticOutput;
use colored::Colorize;
use funtonic::config::CommanderConfig;
use funtonic::crypto::signed_payload::encode_and_sign;
use funtonic::executor_meta::ExecutorMeta;
use funtonic::task_server::{AdminDroppedExecutorJsonResponse, AdminListExecutorKeysJsonResponse};
use grpc_service::grpc_protocol::admin_request::RequestType;
use grpc_service::grpc_protocol::admin_request_response::ResponseKind;
use grpc_service::grpc_protocol::commander_service_client::CommanderServiceClient;
use grpc_service::grpc_protocol::{AdminRequest, Empty};
use prettytable::format::consts::*;
use prettytable::*;
use std::collections::BTreeMap;
use std::str::FromStr;
use structopt::StructOpt;
use tokio::time::Duration;
use tonic::transport::Channel;

#[derive(StructOpt, Debug)]
#[structopt(rename_all = "kebab")]
pub enum AdminCommand {
    /// Get connected executors and their meta as json
    ListConnectedExecutors { query: Option<String> },
    /// Get all known executors and their meta as json
    ListKnownExecutors { query: Option<String> },
    /// Get all running tasks as json
    ListRunningTasks,
    /// Remove the executor from the taskserver
    ///
    /// Remove the executor from the taskserver database, close drop the communication channel if present
    /// this should trigger a reconnect of the executor, and thus an update of the executor's metadata
    /// If the executor is not alive it will be forgotten.
    DropExecutor {
        /// the client_id of the executor to drop
        query: String,
    },
    /// List executors public keys
    ListExecutorKeys,
    /// Approve an executor public key (*) can be used to approve all pending keys
    ApproveExecutorKey { executor: String },
}

#[derive(thiserror::Error, Debug)]
#[error("Output mode must be one of json, pretty-json or human-readable")]
pub struct InvalidOutputMode;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AdminCommandOuputMode {
    Json,
    PrettyJson,
    HumanReadableShort,
    HumanReadableLong,
}

impl FromStr for AdminCommandOuputMode {
    type Err = InvalidOutputMode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use AdminCommandOuputMode::*;
        match s {
            "json" | "js" => Ok(Json),
            "pretty-json" | "pjs" => Ok(PrettyJson),
            "human-readable" | "hr" => Ok(HumanReadableShort),
            "human-readable-long" | "hrl" => Ok(HumanReadableLong),
            _ => Err(InvalidOutputMode),
        }
    }
}

impl AdminCommand {
    fn display_formatted_output(
        &self,
        raw_json: &str,
        output_mode: AdminCommandOuputMode,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match output_mode {
            AdminCommandOuputMode::Json => println!("{}", raw_json),
            AdminCommandOuputMode::PrettyJson => println!(
                "{}",
                // deserialize & serialize to pretty printed json
                serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(
                    &raw_json
                )?)?
            ),
            AdminCommandOuputMode::HumanReadableLong
            | AdminCommandOuputMode::HumanReadableShort => match self {
                AdminCommand::ListConnectedExecutors { query }
                | AdminCommand::ListKnownExecutors { query } => {
                    println!(
                        "Executors matching query: {}",
                        query.as_ref().unwrap_or(&"*".to_string())
                    );
                    let executors: BTreeMap<String, ExecutorMeta> =
                        serde_json::from_str(&raw_json)?;
                    if executors.len() > 0 {
                        let mut table = Table::new();
                        if output_mode == HumanReadableShort {
                            table.set_format(*FORMAT_NO_BORDER_LINE_SEPARATOR);
                        }
                        table.set_titles(row!["client_id", "version", "meta"]);
                        for (client_id, meta) in &executors {
                            let version = meta.version();
                            let meta = if output_mode == HumanReadableShort {
                                serde_json::to_string(&meta.tags())?
                            } else {
                                serde_yaml::to_string(&meta.tags())?[4..].to_string()
                            };
                            table.add_row(row![client_id.green(), version, meta]);
                        }
                        table.printstd();
                        println!("Found {} executors", executors.len().to_string().green());
                    } else {
                        println!("Found {} executor", "0".red());
                    }
                }
                AdminCommand::ListRunningTasks => {
                    let tokens: Vec<String> = serde_json::from_str(&raw_json)?;
                    for token in tokens {
                        println!("{}", token);
                    }
                }
                AdminCommand::DropExecutor { query } => {
                    let dropped_executors: BTreeMap<String, AdminDroppedExecutorJsonResponse> =
                        serde_json::from_str(&raw_json)?;
                    println!("Executors matching query: {}", query);
                    if dropped_executors.len() > 0 {
                        let mut table = Table::new();
                        if output_mode == HumanReadableShort {
                            table.set_format(*FORMAT_NO_BORDER_LINE_SEPARATOR);
                        }
                        table.set_titles(row!["client_id", "known", "connected"]);
                        for (client_id, dropped_status) in &dropped_executors {
                            table.add_row(row![
                                client_id.green(),
                                colored_bool(dropped_status.removed_from_known),
                                colored_bool(dropped_status.removed_from_connected)
                            ]);
                        }
                        table.printstd();
                        println!(
                            "Dropped {} executors",
                            dropped_executors.len().to_string().green()
                        );
                    } else {
                        println!("Found {} executor, none dropped!", "0".red());
                    }
                }
                AdminCommand::ListExecutorKeys => {
                    let keys: AdminListExecutorKeysJsonResponse = serde_json::from_str(&raw_json)?;

                    println!("{}", "Trusted executors".green());
                    let mut table = Table::new();
                    table.set_format(*FORMAT_NO_BORDER_LINE_SEPARATOR);
                    table.set_titles(row!["client_id", "key"]);
                    for (client_id, key) in &keys.trusted_executor_keys {
                        table.add_row(row![client_id.green(), key]);
                    }
                    table.printstd();

                    println!("{}", "Waiting for approval executors".red());
                    let mut table = Table::new();
                    table.set_format(*FORMAT_NO_BORDER_LINE_SEPARATOR);
                    table.set_titles(row!["client_id", "key"]);
                    for (client_id, key) in &keys.unapproved_executor_keys {
                        table.add_row(row![client_id.red(), key]);
                    }
                    table.printstd();
                }
                AdminCommand::ApproveExecutorKey {
                    executor: _executor,
                } => {}
            },
        }

        Ok(())
    }
}

fn colored_bool(b: bool) -> String {
    match b {
        true => format!("{}", "true".green()),
        false => format!("{}", "false".red()),
    }
}

pub async fn handle_admin_command(
    mut client: CommanderServiceClient<Channel>,
    commander_config: &CommanderConfig,
    admin_command: AdminCommand,
    output_mode: AdminCommandOuputMode,
) -> Result<CommanderSyntheticOutput, Box<dyn std::error::Error>> {
    // grpc prost typing is just awful piece of crap.
    let request = match &admin_command {
        AdminCommand::ListConnectedExecutors { query } => AdminRequest {
            request_type: Some(RequestType::ListConnectedExecutors(
                query.clone().unwrap_or("*".into()),
            )),
        },
        AdminCommand::ListKnownExecutors { query } => AdminRequest {
            request_type: Some(RequestType::ListKnownExecutors(
                query.clone().unwrap_or("*".into()),
            )),
        },
        AdminCommand::ListRunningTasks => AdminRequest {
            request_type: Some(RequestType::ListRunningTasks(Empty {})),
        },

        AdminCommand::DropExecutor { ref query } => AdminRequest {
            request_type: Some(RequestType::DropExecutor(query.clone())),
        },
        AdminCommand::ListExecutorKeys => AdminRequest {
            request_type: Some(RequestType::ListExecutorKeys(Empty {})),
        },
        AdminCommand::ApproveExecutorKey { executor } => AdminRequest {
            request_type: Some(RequestType::ApproveExecutorKey(executor.clone())),
        },
    };

    let request = tonic::Request::new(encode_and_sign(
        request,
        &commander_config.ed25519_key,
        Duration::from_secs(60),
    )?);

    let response = client.admin(request).await?.into_inner();
    match response.response_kind.unwrap() {
        ResponseKind::Error(e) => {
            eprintln!("{}", e.red());
            std::process::exit(1);
        }
        ResponseKind::JsonResponse(j) => {
            admin_command.display_formatted_output(&j, output_mode)?;
            Ok(CommanderSyntheticOutput::Admin(j))
        }
    }
}
