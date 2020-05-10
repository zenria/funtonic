use crate::ExecutorState;
use atty::Stream;
use colored::{Color, Colorize};
use funtonic::config::CommanderConfig;
use funtonic::CLIENT_TOKEN_HEADER;
use grpc_service::grpc_protocol::commander_service_client::CommanderServiceClient;
use grpc_service::grpc_protocol::launch_task_request::Task;
use grpc_service::grpc_protocol::launch_task_response::TaskResponse;
use grpc_service::grpc_protocol::task_execution_result::ExecutionResult;
use grpc_service::grpc_protocol::task_output::Output;
use grpc_service::grpc_protocol::{ExecuteCommand, LaunchTaskRequest};
use indicatif::ProgressBar;
use query_parser::parse;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use structopt::StructOpt;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

#[derive(StructOpt, Debug)]
pub struct Cmd {
    /// Raw output, remote stderr/out will be printed as soon at they arrive without any other information
    #[structopt(short = "r", long = "raw")]
    pub raw: bool,
    /// Group output by executor instead displaying a live stream of all executor outputs
    #[structopt(short = "g", long = "group")]
    pub group: bool,
    /// Do not display the progress bar, note that is will be hidden if stderr is not a tty
    #[structopt(short = "n", long = "no-progress")]
    pub no_progress: bool,
    /// testing opt
    #[structopt(long = "no_std_process_return")]
    pub no_std_process_return: bool,
    pub query: String,
    pub command: Vec<String>,
}

pub async fn handle_cmd(
    mut client: CommanderServiceClient<Channel>,
    commander_config: &CommanderConfig,
    cmd: Cmd,
) -> Result<(), Box<dyn Error>> {
    let Cmd {
        raw,
        group,
        no_progress,
        query,
        command,
        no_std_process_return,
    } = cmd;
    //check the query is parsable
    parse(&query)?;
    let command = command.join(" ");

    let mut request = tonic::Request::new(LaunchTaskRequest {
        task: Some(Task::ExecuteCommand(ExecuteCommand { command })),
        predicate: query.clone(),
    });
    request.metadata_mut().append(
        CLIENT_TOKEN_HEADER,
        MetadataValue::from_str(&commander_config.client_token).unwrap(),
    );

    let mut response = client.launch_task(request).await?.into_inner();

    let mut executors = HashMap::new();

    // output by executor
    let mut executors_output = HashMap::new();

    let mut pb: Option<ProgressBar> = None;

    while let Some(task_execution_result) = response.message().await? {
        debug!("Received {:?}", task_execution_result);
        // by convention this field is always here, so we can "safely" unwrap
        let task_response = task_execution_result.task_response.unwrap();
        match task_response {
            TaskResponse::MatchingExecutors(mut e) => {
                e.client_id.sort();
                if !raw {
                    let executors_string = e.client_id.join(", ");
                    if no_progress || !atty::is(Stream::Stdout) {
                        println!("Matching executors: {}", executors_string);
                    } else {
                        let progress = ProgressBar::new(e.client_id.len() as u64);
                        progress.println(format!("Matching executors: {}", executors_string));
                        pb = Some(progress);
                    }
                    for id in e.client_id {
                        executors.insert(id, ExecutorState::Matching);
                    }
                }
            }
            TaskResponse::TaskExecutionResult(task_execution_result) => {
                let client_id = &task_execution_result.client_id;
                match task_execution_result.execution_result.unwrap() {
                    ExecutionResult::TaskAborted(_) => {
                        debug!("Tasks completed on {} (KILLED)", client_id);
                        *executors
                            .entry(client_id.clone())
                            .or_insert(ExecutorState::Matching) = ExecutorState::Error;
                        if let Some(pb) = &pb {
                            pb.inc(1);
                        }
                        if group && !raw {
                            if let Some(lines) = executors_output.remove(client_id) {
                                match &pb {
                                    None => {
                                        println!("{} {}:", "########".green(), client_id);
                                        for line in lines {
                                            println!("{}", line);
                                        }
                                    }
                                    Some(pb) => {
                                        pb.println(format!(
                                            "{} {}:",
                                            "########".green(),
                                            client_id
                                        ));
                                        for line in lines {
                                            pb.println(line);
                                        }
                                    }
                                }
                            }
                        }
                    }
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
                        if !raw {
                            if let Some(pb) = &pb {
                                pb.inc(1);
                            }
                            if group {
                                if let Some(lines) = executors_output.remove(client_id) {
                                    match &pb {
                                        None => {
                                            println!("{} {}:", "########".green(), client_id);
                                            for line in lines {
                                                println!("{}", line);
                                            }
                                        }
                                        Some(pb) => {
                                            pb.println(format!(
                                                "{} {}:",
                                                "########".green(),
                                                client_id
                                            ));
                                            for line in lines {
                                                pb.println(line);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ExecutionResult::TaskOutput(output) => {
                        if let Some(output) = output.output.as_ref() {
                            if raw {
                                match output {
                                    Output::Stdout(o) => println!("{}", o),
                                    Output::Stderr(e) => eprintln!("{}", e),
                                }
                            } else if group {
                                (*executors_output
                                    .entry(client_id.clone())
                                    .or_insert(Vec::new()))
                                .push(match output {
                                    Output::Stdout(o) => o.clone(),
                                    Output::Stderr(e) => format!("{}", e.trim_end().red()),
                                });
                            } else {
                                let out = match output {
                                    Output::Stdout(o) => {
                                        format!("{}: {}", client_id.green(), o.trim_end())
                                    }
                                    Output::Stderr(e) => {
                                        format!("{}: {}", client_id.red(), e.trim_end())
                                    }
                                };
                                match &pb {
                                    None => println!("{}", out),
                                    Some(pb) => pb.println(out),
                                }
                            }
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
                        pb.iter().for_each(|pb| {
                            pb.println(format!("{} disconnected!", client_id.red()))
                        });
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
    if executors.len() == 0 {
        success = false;
    }
    let mut states = BTreeMap::new();
    for (client_id, state) in executors {
        if state != ExecutorState::Success {
            success = false;
        }
        (*states.entry(state).or_insert(BTreeSet::new())).insert(client_id);
    }
    if !raw {
        for (state, client_ids) in states {
            println!("{}: {}", state, colorize(client_ids.iter(), state.color()));
        }
    }
    if no_std_process_return {
        Ok(())
    } else {
        std::process::exit(if success { 0 } else { 1 });
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
