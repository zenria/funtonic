use crate::executor_meta::ExecutorMeta;
use crate::{CLIENT_TOKEN_HEADER, PROTOCOL_VERSION};
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use grpc_service::grpc_protocol::admin_request::RequestType;
use grpc_service::grpc_protocol::admin_request_response::ResponseKind;
use grpc_service::grpc_protocol::launch_task_request::Task;
use grpc_service::grpc_protocol::launch_task_response::TaskResponse;
use grpc_service::grpc_protocol::task_execution_result::ExecutionResult;
use grpc_service::grpc_protocol::tasks_manager_server::*;
use grpc_service::grpc_protocol::*;
use query_parser::{parse, Query, QueryMatcher};
use rand::Rng;
use rustbreak::deser::Yaml;
use rustbreak::{Database, FileDatabase};
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::{Code, Request, Response, Status, Streaming};

#[derive(Debug, Error)]
#[error("Rustbreak database error {0}")]
struct RustBreakWrappedError(rustbreak::RustbreakError);

pub struct TaskServer {
    /// executors by id: when a task must be submited to an executor,
    /// a Sender is sent to each matching executor
    executors: Mutex<
        HashMap<
            String,
            mpsc::UnboundedSender<(ExecuteCommand, mpsc::UnboundedSender<TaskResponse>)>,
        >,
    >,

    /// by task id, sinks where executors reports task execution
    tasks_sinks: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TaskResponse>>>>,

    executor_meta_database: Arc<FileDatabase<HashMap<String, ExecutorMeta>, Yaml>>,

    /// map<token, name> the name is used for logging purpose
    authorized_client_tokens: BTreeMap<String, String>,
}

impl TaskServer {
    pub fn new(
        database_path: &Path,
        authorized_client_tokens: BTreeMap<String, String>,
    ) -> Result<Self, anyhow::Error> {
        if !database_path.exists() {
            let mut empty = File::create(database_path)?;
            empty.write("---\n{}".as_bytes())?;
        }

        let db = FileDatabase::from_path(database_path, Default::default())
            .map_err(|e| RustBreakWrappedError(e))?;
        db.load().map_err(|e| RustBreakWrappedError(e))?;
        Ok(TaskServer {
            executors: Mutex::new(HashMap::new()),
            tasks_sinks: Arc::new(Mutex::new(HashMap::new())),
            executor_meta_database: Arc::new(db),
            authorized_client_tokens,
        })
    }
}

fn register_new_task(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskResponse>>>,
    sender_to_commander: mpsc::UnboundedSender<TaskResponse>,
) -> String {
    let task_id = random_task_id();
    tasks_sinks
        .lock()
        .unwrap()
        .insert(task_id.clone(), sender_to_commander);
    task_id
}

fn get_task_sink(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskResponse>>>,
    task_id: &str,
) -> Option<mpsc::UnboundedSender<TaskResponse>> {
    tasks_sinks.lock().unwrap().remove(task_id)
}

impl TaskServer {
    fn get_channels_to_matching_executors(
        &self,
        query: &Query,
    ) -> Result<
        Vec<(
            String,
            Option<mpsc::UnboundedSender<(ExecuteCommand, mpsc::UnboundedSender<TaskResponse>)>>,
        )>,
        RustBreakWrappedError,
    > {
        let client_ids: Vec<String> = self
            .executor_meta_database
            .read(|executors| {
                executors
                    .iter()
                    .filter(|(_client_id, meta)| meta.qmatches(query))
                    .map(|(client_id, _meta)| client_id.clone())
                    .collect()
            })
            .map_err(|e| RustBreakWrappedError(e))?;

        let executor_senders = self.executors.lock().unwrap();
        // find matching senders, clone them
        Ok(client_ids
            .into_iter()
            .map(|client_id| {
                let executor_sender = executor_senders
                    .get(&client_id)
                    .map(|executor_sender| executor_sender.clone());
                (client_id, executor_sender)
            })
            .collect())
    }

    fn register_executor(
        &self,
        executor_meta: ExecutorMeta,
        sender_to_get_task_response: mpsc::UnboundedSender<(
            ExecuteCommand,
            mpsc::UnboundedSender<TaskResponse>,
        )>,
    ) -> Result<(), RustBreakWrappedError> {
        self.executors.lock().unwrap().insert(
            executor_meta.client_id().to_string(),
            sender_to_get_task_response,
        );

        self.executor_meta_database
            .write(move |executors| {
                info!(
                    "Registered {}",
                    serde_yaml::to_string(&executor_meta).unwrap_or("???".to_string())
                );
                executors.insert(executor_meta.client_id().to_string(), executor_meta);
            })
            .map_err(|e| RustBreakWrappedError(e))?;
        self.executor_meta_database
            .save()
            .map_err(|e| RustBreakWrappedError(e))
    }
}

type Stream<T> =
    Pin<Box<dyn futures::Stream<Item = std::result::Result<T, Status>> + Send + Sync + 'static>>;

#[tonic::async_trait]
impl TasksManager for TaskServer {
    type GetTasksStream = Stream<GetTaskStreamReply>;
    async fn get_tasks(
        &self,
        request: tonic::Request<GetTasksRequest>,
    ) -> Result<tonic::Response<Self::GetTasksStream>, tonic::Status> {
        let metadata = request.metadata();
        let request = request.get_ref();

        // Strict protocol version check
        if request.client_protocol_version != PROTOCOL_VERSION {
            warn!(
                "{} has protocol version {}, but expecting protovol version {}",
                request.client_id, request.client_protocol_version, PROTOCOL_VERSION
            );
            Err(tonic::Status::new(
                Code::FailedPrecondition,
                format!(
                    "Expecting protocol version {} but got {}",
                    PROTOCOL_VERSION, request.client_protocol_version
                ),
            ))?;
        }

        let client_id = request.client_id.clone();
        info!("{} connected with meta {:?}", client_id, metadata);
        // register the client and wait for new tasks to come, forward them
        // to the response
        let (sender, receiver) = mpsc::unbounded();
        if let Err(e) = self.register_executor(request.into(), sender) {
            error!("Unable to register executor {}", e);
            Err(tonic::Status::new(
                Code::Internal,
                "internal storage error!",
            ))?;
        }

        let tasks_sinks = self.tasks_sinks.clone();

        let response_stream = receiver.map(move |(execute_command, sender_to_commander)| {
            // for each new task, register the task and forward it to the executor stream
            let task_id = register_new_task(&tasks_sinks, sender_to_commander);
            info!(
                "Sending task {} - {:?} to {}",
                task_id, execute_command, client_id
            );
            Ok(GetTaskStreamReply {
                task_id,
                execute_command: Some(execute_command),
            })
        });

        Ok(Response::new(
            Box::pin(response_stream) as Self::GetTasksStream
        ))
    }
    async fn task_execution(
        &self,
        request: tonic::Request<tonic::Streaming<TaskExecutionResult>>,
    ) -> Result<tonic::Response<Empty>, tonic::Status> {
        let task_id =
            String::from_utf8_lossy(request.metadata().get("task_id").unwrap().as_bytes())
                .into_owned();
        let mut request_stream = request.into_inner();
        if let Some(sender) = get_task_sink(&self.tasks_sinks, &task_id) {
            let mut sender = sender;
            while let Some(task_execution_stream) = request_stream.next().await {
                let task_execution_stream = task_execution_stream?;
                debug!(
                    "Received task_execution_report {} - {}",
                    task_execution_stream.client_id, task_id
                );
                if let Some(execution_result) = &task_execution_stream.execution_result {
                    if let ExecutionResult::TaskAborted(_) = execution_result {
                        info!(
                            "Task {} aborted (killed) on {}",
                            task_id, task_execution_stream.client_id,
                        );
                    }
                    if let ExecutionResult::TaskCompleted(completed) = execution_result {
                        info!(
                            "Task {} completed with code {} on {}",
                            task_id, task_execution_stream.client_id, completed.return_code
                        );
                    }
                }
                if let Err(_e) = sender
                    .send(TaskResponse::TaskExecutionResult(task_execution_stream))
                    .await
                {
                    warn!(
                        "Commander disconnected for task {}, task will be killed by executor if not already done.",
                        task_id
                    );
                    break;
                }
            }
            Ok(Response::new(Empty {}))
        } else {
            error!("Task id not found {}", task_id);
            Err(tonic::Status::new(Code::NotFound, "task_id not found"))
        }
    }
    type LaunchTaskStream = Stream<LaunchTaskResponse>;

    async fn launch_task(
        &self,
        request: tonic::Request<LaunchTaskRequest>,
    ) -> Result<tonic::Response<Self::LaunchTaskStream>, tonic::Status> {
        let token_name = self.verify_token(&request)?;

        let request = request.get_ref();
        let query = &request.predicate;
        let command = match request
            .task
            .as_ref()
            .ok_or(Status::invalid_argument("Missing task"))?
        {
            Task::ExecuteCommand(command) => command,
            Task::StreamingPayload(_) => {
                return Err(Status::new(Code::Internal, "not implemented"))
            }
        };
        //let payload = request.task_payload.as_ref().unwrap();

        // this channel will be sent to the matching executors. the executors will then register it so
        // further task progression reporting could be sent o
        let (mut sender, receiver) = mpsc::unbounded::<TaskResponse>();

        info!(
            "Command received {:?} for {} with token {}",
            command, query, token_name
        );

        let query = parse(query).map_err(|parse_error| {
            Status::invalid_argument(format!("Invalid query: {}", parse_error))
        })?;
        debug!("Parsed query: {:#?}", query);

        let mut senders = self
            .get_channels_to_matching_executors(&query)
            .map_err(|e| tonic::Status::new(Code::Internal, format!("Unexpected Error {}", e)))?;

        let matching_clients: Vec<String> = senders
            .iter()
            .map(|(client_id, _)| client_id.clone())
            .collect();

        sender
            .send(TaskResponse::MatchingExecutors(MatchingExecutors {
                client_id: matching_clients,
            }))
            .await
            .map_err(|e| {
                error!("Commander disconnected!");
                tonic::Status::new(Code::Internal, format!("Unexpected Error {}", e))
            })?;

        for (client_id, executor_sender) in senders.iter_mut() {
            debug!("client {} matches query!", client_id);
            if let Some(executor_sender) = executor_sender {
                match executor_sender
                    .send((command.clone(), sender.clone()))
                    .await
                {
                    Err(_) => {
                        // disconnected executor: task sink has been found
                        error!("Executor {} disconnected!", client_id);
                        sender
                            .send(TaskResponse::TaskExecutionResult(TaskExecutionResult {
                                task_id: random_task_id(),
                                client_id: client_id.clone(),
                                execution_result: Some(ExecutionResult::Disconnected(Empty {})),
                            }))
                            .await
                            .map_err(|e| {
                                error!("Commander disconnected!");
                                tonic::Status::new(
                                    Code::Internal,
                                    format!("Unexpected Error {}", e),
                                )
                            })?;
                    }
                    Ok(..) => {
                        info!("Command {:?} sent to {}", command, client_id);
                        sender
                            .send(TaskResponse::TaskExecutionResult(TaskExecutionResult {
                                task_id: random_task_id(),
                                client_id: client_id.clone(),
                                execution_result: Some(ExecutionResult::TaskSubmitted(Empty {})),
                            }))
                            .await
                            .map_err(|e| {
                                error!("Commander disconnected!");
                                tonic::Status::new(
                                    Code::Internal,
                                    format!("Unexpected Error {}", e),
                                )
                            })?;
                    }
                }
            } else {
                // executor is knowm but no commication channel has been found
                sender
                    .send(TaskResponse::TaskExecutionResult(TaskExecutionResult {
                        task_id: random_task_id(),
                        client_id: client_id.clone(),
                        execution_result: Some(ExecutionResult::Disconnected(Empty {})),
                    }))
                    .await
                    .map_err(|e| {
                        error!("Commander disconnected!");
                        tonic::Status::new(Code::Internal, format!("Unexpected Error {}", e))
                    })?;
            }
        }
        let response_stream = receiver.map(|task_response| {
            Ok(LaunchTaskResponse {
                task_response: Some(task_response),
            })
        });
        Ok(Response::new(
            Box::pin(response_stream) as Self::LaunchTaskStream
        ))
    }

    async fn admin(
        &self,
        request: Request<AdminRequest>,
    ) -> Result<Response<AdminRequestResponse>, Status> {
        let token_name = self.verify_token(&request)?;

        let request = request.into_inner();
        info!("{} - {:?}", token_name, request);

        match request
            .request_type
            .ok_or(Status::invalid_argument("Missing request type"))?
        {
            RequestType::ListConnectedExecutors(_) => {
                let connected_executors = self
                    .executors
                    .lock()
                    .map_err(|_| Status::internal("Unable to lock"))?
                    .iter()
                    .map(|(client_id, _)| client_id.clone())
                    .collect::<HashSet<_>>();
                Ok(Response::new(AdminRequestResponse {
                    response_kind: Some(ResponseKind::JsonResponse(
                        self.executor_meta_database
                            .read(|data| {
                                serde_json::to_string(
                                    &data
                                        .iter()
                                        .filter(|(client_id, _)| {
                                            connected_executors.contains(*client_id)
                                        })
                                        .collect::<HashMap<_, _>>(),
                                )
                            })
                            .map_err(|e| {
                                Status::internal(format!("Unable to read database {}", e))
                            })?
                            .map_err(|deser| {
                                Status::internal(format!("An error occured: {}", deser))
                            })?,
                    )),
                }))
            }
            RequestType::ListKnownExecutors(_) => Ok(Response::new(AdminRequestResponse {
                response_kind: Some(ResponseKind::JsonResponse(
                    self.executor_meta_database
                        .read(|data| serde_json::to_string(data))
                        .map_err(|e| Status::internal(format!("Unable to read database {}", e)))?
                        .map_err(|deser| {
                            Status::internal(format!("An error occured: {}", deser))
                        })?,
                )),
            })),
            RequestType::ListRunningTasks(_) => Ok(Response::new(AdminRequestResponse {
                response_kind: Some(ResponseKind::JsonResponse(
                    serde_json::to_string(
                        &self
                            .tasks_sinks
                            .lock()
                            .map_err(|_| Status::internal("Unable to lock"))?
                            .iter()
                            .map(|(task_id, _)| task_id)
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|deser| Status::internal(format!("An error occured: {}", deser)))?,
                )),
            })),
            RequestType::ListTokens(_) => Ok(Response::new(AdminRequestResponse {
                response_kind: Some(ResponseKind::JsonResponse(
                    serde_json::to_string(
                        &self
                            .authorized_client_tokens
                            .iter()
                            .map(|(_, token)| token)
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|deser| Status::internal(format!("An error occured: {}", deser)))?,
                )),
            })),
            RequestType::DropExecutor(client_id) => {
                // remove from database
                let removed_from_known = self
                    .executor_meta_database
                    .write(|data| data.remove(&client_id).is_some())
                    .map_err(|e| Status::internal(format!("Unable to read database {}", e)))?;
                // remove from connected executors
                let removed_from_connected = self
                    .executors
                    .lock()
                    .map_err(|_| Status::internal("Unable to lock"))?
                    .remove(&client_id)
                    .is_some();
                let ret = AdminDropExecutorJsonResponse {
                    removed_from_connected,
                    removed_from_known,
                };
                Ok(Response::new(AdminRequestResponse {
                    response_kind: Some(ResponseKind::JsonResponse(
                        serde_json::to_string(&ret).map_err(|deser| {
                            Status::internal(format!("An error occured: {}", deser))
                        })?,
                    )),
                }))
            }
        }
    }
}

impl TaskServer {
    fn verify_token<T>(&self, request: &Request<T>) -> Result<&String, Status> {
        let metadata = request.metadata();
        let token = metadata.get(CLIENT_TOKEN_HEADER);
        match token {
            None => Err(Status::new(Code::PermissionDenied, "no client token")),
            Some(token) => {
                let token = token.to_str().map_err(|e| {
                    Status::new(
                        Code::PermissionDenied,
                        format!("invalid client token: {}", e),
                    )
                })?;
                self.authorized_client_tokens
                    .get(token)
                    .ok_or(Status::new(Code::PermissionDenied, "invalid client token"))
            }
        }
    }
}

fn random_task_id() -> String {
    let id: u128 = rand::thread_rng().gen();
    format!("{:x}", id)
}

#[derive(Serialize, Deserialize)]
pub struct AdminDropExecutorJsonResponse {
    pub removed_from_connected: bool,
    pub removed_from_known: bool,
}
