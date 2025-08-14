use crate::json::JsonCollector;
use crate::{CommanderSyntheticOutput, ExecutorState};
use anyhow::{anyhow, Context};
use atty::Stream;
use clap::{Args, Subcommand, ValueEnum};
use colored::{Color, Colorize};
use directories::ProjectDirs;
use funtonic::config::CommanderConfig;
use funtonic::crypto::signed_payload::encode_and_sign;
use funtonic::data_encoding;
use funtonic::tonic::{self, Request};
use grpc_service::grpc_protocol::commander_service_client::CommanderServiceClient;
use grpc_service::grpc_protocol::launch_task_request_payload::Task;
use grpc_service::grpc_protocol::launch_task_response::TaskResponse;
use grpc_service::grpc_protocol::task_execution_result::ExecutionResult;
use grpc_service::grpc_protocol::task_output::Output;
use grpc_service::grpc_protocol::{
    ExecuteCommand, LaunchTaskRequest, LaunchTaskRequestPayload, PublicKey,
};
use indicatif::ProgressBar;
use query_parser::parse;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use shellish_parse::ParseOptions;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;
use tonic::transport::Channel;

#[derive(Args, Debug, Clone, Default)]
pub struct CommandOptions {
    /// Raw output, remote stderr/out will be printed as soon at they arrive without any other information
    #[arg(short = 'r', long = "raw")]
    pub raw: bool,
    /// Output exec result as json. Keys are hosts, value is stdout.
    #[arg(short, long)]
    pub json: bool,
    #[arg(long, default_value = "escape-separate")]
    pub json_mode: JsonMode,
    /// Group output by executor instead displaying a live stream of all executor outputs
    #[arg(short = 'g', long = "group")]
    pub group: bool,
    /// Do not display the progress bar, note that is will be hidden if stderr is not a tty
    #[arg(short = 'n', long = "no-progress")]
    pub no_progress: bool,
    /// testing opt
    #[arg(long = "no_std_process_return")]
    pub no_std_process_return: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, Default)]
pub enum JsonMode {
    /// includes both stdout & stderr as string into a json object `{"stdout": "...", "stderr": "..."}`
    #[default]
    EscapeSeparate,
    /// includes both stdout & stderr as string into a single string
    EscapeMerge,
    /// treat stdout as a valid json object, ignores stderr (stderr will be FW to stderr)
    StdoutJson,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Run a command on targeted executors
    #[command(name = "run")]
    Run {
        #[command(flatten)]
        options: CommandOptions,
        /// Target query
        query: String,
        command: Vec<String>,
    },
    /// Run commands in interactive mode
    #[command(name = "int")]
    Int {
        #[command(flatten)]
        options: CommandOptions,
        /// Target query
        query: String,
    },
    /// Manage authorized keys on executors
    #[command(name = "keys")]
    Keys {
        #[command(flatten)]
        options: CommandOptions,
        /// Target query: All matching executor will install the specified key in their
        /// authorized key.
        query: String,
        #[command(subcommand)]
        key_cmd: KeyCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum KeyCmd {
    /// Authorize a key on executors
    #[command(name = "authorize")]
    Authorize {
        /// Identifier of the key
        key_id: String,
        /// Public key (base64 encoded)
        public_key: String,
    },
    /// Revoke a key on executors
    #[command(name = "revoke")]
    Revoke {
        // Identifier of the public key on executors
        key_id: String,
    },
}

pub async fn handle_cmd(
    client: CommanderServiceClient<Channel>,
    commander_config: &CommanderConfig,
    cmd: Cmd,
) -> Result<CommanderSyntheticOutput, Box<dyn Error>> {
    if let Cmd::Int { mut options, query } = cmd {
        // interactive mode

        //check the query is parsable
        parse(&query)?;

        // craft a special command to retrieve the list of connected executors
        {
            let mut options = CommandOptions::default();
            options.no_std_process_return = true;
            let request = tonic::Request::new(LaunchTaskRequest {
                payload: Some(encode_and_sign(
                    LaunchTaskRequestPayload {
                        task: Some(Task::ExecuteCommand(ExecuteCommand { command: "".into() })),
                    },
                    &commander_config.ed25519_key,
                    Duration::from_secs(60),
                )?),

                predicate: query.clone(),
            });
            do_handle_cmd(client.clone(), request, options.clone()).await?;
        }

        // do not exit process on return
        options.no_std_process_return = true;

        let mut rl = DefaultEditor::new()?;

        let history_path = match ProjectDirs::from("io", "scoopit", "Funtonic") {
            Some(proj_dirs) => {
                let data_dir = proj_dirs.data_dir();
                let _ = std::fs::create_dir_all(data_dir);
                let mut history_path = PathBuf::from(data_dir);
                history_path.push("interactive_history.txt");
                Some(history_path)
            }
            None => None,
        };
        if let Some(history) = &history_path {
            let _ = rl.load_history(history);
        }
        loop {
            let readline = rl.readline(&format!("{query} > "));

            match readline {
                Ok(line) => {
                    let _ = rl.add_history_entry(line.as_str()); // ignore result

                    if let Err(e) = safeguard_command(&line) {
                        eprintln!("{e}");
                        continue;
                    }

                    let request = tonic::Request::new(LaunchTaskRequest {
                        payload: Some(encode_and_sign(
                            LaunchTaskRequestPayload {
                                task: Some(Task::ExecuteCommand(ExecuteCommand { command: line })),
                            },
                            &commander_config.ed25519_key,
                            Duration::from_secs(60),
                        )?),

                        predicate: query.clone(),
                    });
                    do_handle_cmd(client.clone(), request, options.clone()).await?;
                }
                Err(ReadlineError::Interrupted) => {
                    break;
                }
                Err(ReadlineError::Eof) => {
                    break;
                }
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        }
        if let Some(history) = &history_path {
            let _ = rl.save_history(history);
        }
        std::process::exit(0);
    } else {
        let (request, options) = match cmd {
            Cmd::Run {
                options,
                query,
                command,
            } => {
                //check the query is parsable
                parse(&query)?;
                let command = command.join(" ");

                safeguard_command(&command)?;

                let request = tonic::Request::new(LaunchTaskRequest {
                    payload: Some(encode_and_sign(
                        LaunchTaskRequestPayload {
                            task: Some(Task::ExecuteCommand(ExecuteCommand { command })),
                        },
                        &commander_config.ed25519_key,
                        Duration::from_secs(60),
                    )?),

                    predicate: query,
                });
                (request, options)
            }

            Cmd::Keys {
                options,
                query,
                key_cmd,
            } => {
                //check the query is parsable
                parse(&query)?;
                (
                    match key_cmd {
                        KeyCmd::Authorize { key_id, public_key } => {
                            tonic::Request::new(LaunchTaskRequest {
                                payload: Some(encode_and_sign(
                                    LaunchTaskRequestPayload {
                                        task: Some(Task::AuthorizeKey(PublicKey {
                                            key_id,
                                            key_bytes: data_encoding::BASE64
                                                .decode(public_key.as_bytes())
                                                .context("Unable to decode base64 encoded key")?,
                                        })),
                                    },
                                    &commander_config.ed25519_key,
                                    Duration::from_secs(60),
                                )?),

                                predicate: query,
                            })
                        }
                        KeyCmd::Revoke { key_id } => tonic::Request::new(LaunchTaskRequest {
                            payload: Some(encode_and_sign(
                                LaunchTaskRequestPayload {
                                    task: Some(Task::RevokeKey(key_id)),
                                },
                                &commander_config.ed25519_key,
                                Duration::from_secs(60),
                            )?),

                            predicate: query,
                        }),
                    },
                    options,
                )
            }
            Cmd::Int {
                options: _,
                query: _,
            } => panic!("You should never reach this code"),
        };
        do_handle_cmd(client, request, options).await
    }
}

pub async fn do_handle_cmd(
    mut client: CommanderServiceClient<Channel>,
    request: Request<LaunchTaskRequest>,
    options: CommandOptions,
) -> Result<CommanderSyntheticOutput, Box<dyn Error>> {
    let CommandOptions {
        raw,
        group,
        no_progress,
        no_std_process_return,
        json,
        json_mode,
    } = options;

    let mut response = client.launch_task(request).await?.into_inner();

    let mut executors = HashMap::new();

    // output by executor
    let mut executors_output = HashMap::new();

    let mut json_collector = JsonCollector::new(json_mode);

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
                        eprintln!("Matching executors: {}", executors_string);
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
                    ExecutionResult::TaskRejected(reason) => {
                        debug!("Tasks completed on {} (REJECTED: {})", client_id, reason);
                        *executors
                            .entry(client_id.clone())
                            .or_insert(ExecutorState::Matching) = ExecutorState::Error;
                        if let Some(pb) = &pb {
                            pb.inc(1);
                        }
                        if group && !raw {
                            match &pb {
                                None => {
                                    println!("{} {}:", "########".green(), client_id);
                                    println!("{}: {}", "Task rejected".red(), reason);
                                }
                                Some(pb) => {
                                    pb.println(format!("{} {}:", "########".green(), client_id));
                                    pb.println(format!("{}: {}", "Task rejected".red(), reason));
                                }
                            }
                        } else {
                            match &pb {
                                None => eprintln!(
                                    "{}: {}: {}",
                                    client_id.red(),
                                    "Task rejected".red(),
                                    reason
                                ),
                                Some(pb) => pb.println(format!(
                                    "{}: {}: {}",
                                    client_id.red(),
                                    "Task rejected".red(),
                                    reason
                                )),
                            }
                        }
                    }

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
                                if let Some(lines) = executors_output.get(client_id) {
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
                            if json {
                                match output {
                                    Output::Stdout(d) => {
                                        json_collector.collect_stdout(&client_id, d.clone())
                                    }
                                    Output::Stderr(d) => {
                                        json_collector.collect_stderr(&client_id, d.clone())
                                    }
                                }
                            } else if raw {
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
        for (state, client_ids) in &states {
            eprintln!("{}: {}", state, colorize(client_ids.iter(), state.color()));
        }
    }
    if json {
        println!("{}", json_collector.into_json().to_string())
    }
    if no_std_process_return {
        Ok(CommanderSyntheticOutput::Executor {
            states,
            output: executors_output,
        })
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

/// This will prompt something if an unsafe command is run from a terminal with a tty input
///
/// Unsafe means commands like 'reboot', 'rm'.
///
/// It will return an error if the user do not agree to run the command
fn safeguard_command(command: &str) -> anyhow::Result<()> {
    let Ok(parsed_commands) = shellish_parse::multiparse(
        command,
        ParseOptions::default(),
        &["&&", "||", "&", "|", ";"],
    ) else {
        return Ok(());
    };
    for command in parsed_commands {
        let Some(command) = command.0.get(0) else {
            return Ok(());
        };
        if command.ends_with("reboot") || command.ends_with("rm") || command.ends_with("halt") {
            if atty::isnt(Stream::Stdin) {
                eprintln!("stdin not a tty, running unsafe command {command} anyway!");
                return Ok(());
            }
            // unsafe command
            let mut rl = DefaultEditor::new()?;
            let prompt = format!("Do you really want to run unsafe command `{command}` (y/N)? ");
            loop {
                let line = rl.readline(&prompt)?;
                if line.eq_ignore_ascii_case("y") || line.eq_ignore_ascii_case("yes") {
                    return Ok(());
                } else {
                    return Err(anyhow!("Cancelled!"));
                }
            }
        }
    }
    Ok(())
}
