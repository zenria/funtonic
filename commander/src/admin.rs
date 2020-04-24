use colored::Colorize;
use funtonic::config::CommanderConfig;
use funtonic::executor_meta::ExecutorMeta;
use funtonic::task_server::AdminDropExecutorJsonResponse;
use funtonic::CLIENT_TOKEN_HEADER;
use grpc_service::grpc_protocol::admin_request::RequestType;
use grpc_service::grpc_protocol::admin_request_response::ResponseKind;
use grpc_service::grpc_protocol::tasks_manager_client::TasksManagerClient;
use grpc_service::grpc_protocol::{AdminRequest, Empty};
use std::collections::BTreeMap;
use std::str::FromStr;
use structopt::StructOpt;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

#[derive(StructOpt, Debug)]
#[structopt(rename_all = "kebab")]
pub enum AdminCommand {
    /// Get connected executors and their meta as json
    ListConnectedExecutors,
    /// Get all known executors and their meta as json
    ListKnownExecutors,
    /// Get all running tasks as json
    ListRunningTasks,
    /// List all accepted tokens
    ListTokens,
    /// Remove the executor from the taskserver
    ///
    /// Remove the executor from the taskserver database, close drop the communication channel if present
    /// this should trigger a reconnect of the executor, and thus an update of the executor's metadata
    /// If the executor is not alive it will be forgotten.
    DropExecutor {
        /// the client_id of the executor to drop
        client_id: String,
    },
}

#[derive(thiserror::Error, Debug)]
#[error("Output mode must be one of json, pretty-json or human-readable")]
pub struct InvalidOutputMode;

#[derive(Debug)]
pub enum AdminCommandOuputMode {
    Json,
    PrettyJson,
    HumanReadable,
}

impl FromStr for AdminCommandOuputMode {
    type Err = InvalidOutputMode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use AdminCommandOuputMode::*;
        match s {
            "json" => Ok(Json),
            "pretty-json" => Ok(PrettyJson),
            "human-readable" => Ok(HumanReadable),
            _ => Err(InvalidOutputMode),
        }
    }
}

impl AdminCommand {
    fn display_formatted_output(
        &self,
        raw_json: String,
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
            AdminCommandOuputMode::HumanReadable => match self {
                AdminCommand::ListConnectedExecutors | AdminCommand::ListKnownExecutors => {
                    let executors: BTreeMap<String, ExecutorMeta> =
                        serde_json::from_str(&raw_json)?;
                    for (client_id, meta) in executors {
                        println!(
                            "{}: v{} {}",
                            client_id.green(),
                            meta.version(),
                            serde_json::to_string(&meta.tags())?
                        );
                    }
                }
                AdminCommand::ListRunningTasks => {
                    let tokens: Vec<String> = serde_json::from_str(&raw_json)?;
                    for token in tokens {
                        println!("{}", token);
                    }
                }
                AdminCommand::ListTokens => {
                    let tokens: Vec<String> = serde_json::from_str(&raw_json)?;
                    for token in tokens {
                        println!("{}", token);
                    }
                }
                AdminCommand::DropExecutor { .. } => {
                    let drop_response: AdminDropExecutorJsonResponse =
                        serde_json::from_str(&raw_json)?;
                    println!(
                        "removed_from_known: {}",
                        colored_bool(drop_response.removed_from_known)
                    );
                    println!(
                        "removed_from_connected: {}",
                        colored_bool(drop_response.removed_from_connected)
                    );
                }
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
    mut client: TasksManagerClient<Channel>,
    commander_config: &CommanderConfig,
    admin_command: AdminCommand,
    output_mode: AdminCommandOuputMode,
) -> Result<(), Box<dyn std::error::Error>> {
    // grpc prost typing is just awful piece of crap.
    let request = match admin_command {
        AdminCommand::ListConnectedExecutors => AdminRequest {
            request_type: Some(RequestType::ListConnectedExecutors(Empty {})),
        },
        AdminCommand::ListKnownExecutors => AdminRequest {
            request_type: Some(RequestType::ListKnownExecutors(Empty {})),
        },
        AdminCommand::ListRunningTasks => AdminRequest {
            request_type: Some(RequestType::ListRunningTasks(Empty {})),
        },
        AdminCommand::ListTokens => AdminRequest {
            request_type: Some(RequestType::ListTokens(Empty {})),
        },

        AdminCommand::DropExecutor { ref client_id } => AdminRequest {
            request_type: Some(RequestType::DropExecutor(client_id.clone())),
        },
    };

    let mut request = tonic::Request::new(request);
    request.metadata_mut().append(
        CLIENT_TOKEN_HEADER,
        MetadataValue::from_str(&commander_config.client_token).unwrap(),
    );

    let response = client.admin(request).await?.into_inner();
    match response.response_kind.unwrap() {
        ResponseKind::Error(e) => {
            eprintln!("{}", e.red());
            std::process::exit(1);
        }
        ResponseKind::JsonResponse(j) => admin_command.display_formatted_output(j, output_mode)?,
    }
    Ok(())
}
