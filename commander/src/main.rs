#[macro_use]
extern crate log;

use colored::{Color, ColoredString, Colorize};
use funtonic::config::{Config, Role};
use funtonic::CLIENT_TOKEN_HEADER;
use grpc_service::client::TasksManagerClient;
use grpc_service::launch_task_response::TaskResponse;
use grpc_service::task_execution_result::ExecutionResult;
use grpc_service::task_output::Output;
use grpc_service::{LaunchTaskRequest, TaskPayload};
use http::Uri;
use indicatif::ProgressBar;
use query_parser::parse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Error, Formatter};
use std::io::Stdout;
use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use structopt::StructOpt;
use thiserror::Error;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

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
#[structopt(name = "basic")]
struct Opt {
    #[structopt(short, long, parse(from_os_str))]
    config: Option<PathBuf>,
    query: String,
    command: Vec<String>,
}

#[derive(Error, Debug)]
#[error("Missing field for server config!")]
struct InvalidConfig;

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

    let opt = Opt::from_args();
    let config = Config::parse(&opt.config, "commander.yml")?;
    debug!("Commander starting with config {:#?}", config);
    if let Role::Commander(commander_config) = &config.role {
        let mut channel = Channel::builder(Uri::from_str(&commander_config.server_url)?)
            .tcp_keepalive(Some(Duration::from_secs(60)));
        if let Some(tls_config) = &config.tls {
            channel = channel.tls_config(tls_config.get_client_config()?);
        }
        let channel = channel.connect().await?;

        let mut client = TasksManagerClient::new(channel);

        let command = opt.command.join(" ");

        //check the query is parsable
        parse(&opt.query)?;

        let mut request = tonic::Request::new(LaunchTaskRequest {
            task_payload: Some(TaskPayload { payload: command }),
            predicate: opt.query,
        });
        request.metadata_mut().append(
            CLIENT_TOKEN_HEADER,
            MetadataValue::from_str(&commander_config.client_token).unwrap(),
        );

        let mut response = client.launch_task(request).await?.into_inner();

        let mut executors = HashMap::new();

        let mut pb: Option<ProgressBar> = None;

        while let Some(task_execution_result) = response.message().await? {
            debug!("Received {:?}", task_execution_result);
            // by convention this field is always here, so we can "safely" unwrap
            let task_response = task_execution_result.task_response.unwrap();
            match task_response {
                TaskResponse::MatchingExecutors(mut e) => {
                    e.client_id.sort();
                    let executors_string = e.client_id.join(", ");
                    let progress = ProgressBar::new(e.client_id.len() as u64);
                    progress.println(format!("Matching executors: {}", executors_string));
                    pb = Some(progress);
                    for id in e.client_id {
                        executors.insert(id, ExecutorState::Matching);
                    }
                }
                TaskResponse::TaskExecutionResult(task_execution_result) => {
                    let client_id = &task_execution_result.client_id;
                    match task_execution_result.execution_result.unwrap() {
                        ExecutionResult::TaskCompleted(completion) => {
                            debug!(
                                "Tasks completed on {} with exit code: {}",
                                client_id, completion.return_code
                            );
                            if completion.return_code == 0 {
                                *executors
                                    .entry(client_id.clone())
                                    .or_insert(ExecutorState::Matching) = ExecutorState::Success;
                            } else {
                                *executors
                                    .entry(client_id.clone())
                                    .or_insert(ExecutorState::Matching) = ExecutorState::Error;
                            }
                            pb.iter().for_each(|pb| pb.inc(1))
                        }
                        ExecutionResult::TaskOutput(output) => {
                            if let Some(output) = output.output.as_ref() {
                                pb.iter().for_each(|pb| {
                                    pb.println(match output {
                                        Output::Stdout(o) => {
                                            format!("{}: {}", client_id, o.trim_end().green())
                                        }
                                        Output::Stderr(e) => {
                                            format!("{}: {}", client_id, e.trim_end().red())
                                        }
                                    })
                                });
                            }
                        }
                        ExecutionResult::Ping(_) => {
                            debug!("Pinged!");
                            *executors
                                .entry(client_id.clone())
                                .or_insert(ExecutorState::Matching) = ExecutorState::Alive;
                        }
                        ExecutionResult::Disconnected(_) => {
                            debug!("{} disconnected!", client_id);
                            pb.iter().for_each(|pb| pb.println(format!("{} disconnected!", client_id.red())));
                            *executors
                                .entry(client_id.clone())
                                .or_insert(ExecutorState::Matching) = ExecutorState::Disconnected;
                        }
                        ExecutionResult::TaskSubmitted(_) => {
                            debug!("{} task submitted", client_id);
                            *executors
                                .entry(client_id.clone())
                                .or_insert(ExecutorState::Matching) = ExecutorState::Submitted;
                        }
                    }
                }
            }
        }
        pb.iter().for_each(|pb| pb.finish_and_clear());

        let mut success = true;
        let mut states = BTreeMap::new();
        for (client_id, state) in executors {
            if state != ExecutorState::Success {
                success = false;
            }
            (*states.entry(state).or_insert(BTreeSet::new())).insert(client_id);
        }

        for (state, client_ids) in states {
            println!("{}: {}", state, colorize(client_ids.iter(), state.color()));
        }
        std::process::exit(if success { 0 } else { 1 });
    } else {
        Err(InvalidConfig)?
    }
}

fn colorize<'a, T: Iterator<Item = &'a String>>(collection: T, color: Color) -> String {
    let mut ret = collection.fold(String::new(), |mut acc, item| {
        acc.push_str(&format!("{}, ", item.color(color)));
        acc
    });
    if ret.len() > 2 {
        ret.truncate(ret.len() - 2);
    }
    ret
}
