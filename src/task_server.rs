use crate::executor_meta::ExecutorMeta;
use crate::generated::tasks::server::*;
use crate::generated::tasks::task_execution_result::ExecutionResult;
use crate::generated::tasks::*;
use futures_channel::mpsc;
use futures_sink::Sink;
use futures_util::pin_mut;
use futures_util::{FutureExt, SinkExt, StreamExt};
use rand::Rng;
use rustbreak::deser::Yaml;
use rustbreak::{Database, FileDatabase};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use thiserror::Error;
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
            mpsc::UnboundedSender<(TaskPayload, mpsc::UnboundedSender<TaskExecutionResult>)>,
        >,
    >,

    /// by task id, sinks where executors reports task execution
    tasks_sinks: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionResult>>>>,

    executor_meta_database: Arc<FileDatabase<HashMap<String, ExecutorMeta>, Yaml>>,
}

impl TaskServer {
    pub fn new(database_path: &Path) -> Result<Self, anyhow::Error> {
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
        })
    }
}

fn register_new_task(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionResult>>>,
    sender_to_commander: mpsc::UnboundedSender<TaskExecutionResult>,
) -> String {
    let task_id = random_task_id();
    tasks_sinks
        .lock()
        .unwrap()
        .insert(task_id.clone(), sender_to_commander);
    task_id
}

fn get_task_sink(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionResult>>>,
    task_id: &str,
) -> Option<mpsc::UnboundedSender<TaskExecutionResult>> {
    tasks_sinks.lock().unwrap().remove(task_id)
}

impl TaskServer {
    fn get_channels_to_matching_executors(
        &self,
        query: &str,
    ) -> Result<
        Vec<(
            String,
            Option<
                mpsc::UnboundedSender<(TaskPayload, mpsc::UnboundedSender<TaskExecutionResult>)>,
            >,
        )>,
        RustBreakWrappedError,
    > {
        let client_ids: Vec<String> = self
            .executor_meta_database
            .read(|executors| {
                executors
                    .iter()
                    .filter(|(client_id, meta)| meta.matches(query))
                    .map(|(client_id, _meta)| client_id.clone())
                    .collect()
            })
            .map_err(|e| RustBreakWrappedError(e))?;
        // this code needs to be done in a separate block because aht executors variable is not Send,
        // thus, if it resides in the stack it will fail the whole Future stuff
        //
        let mut executor_senders = self.executors.lock().unwrap();
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
            TaskPayload,
            mpsc::UnboundedSender<TaskExecutionResult>,
        )>,
    ) -> Result<(), RustBreakWrappedError> {
        self.executors.lock().unwrap().insert(
            executor_meta.client_id().to_string(),
            sender_to_get_task_response,
        );

        self.executor_meta_database
            .write(move |executors| {
                executors.insert(executor_meta.client_id().to_string(), executor_meta);
            })
            .map_err(|e| RustBreakWrappedError(e))?;
        self.executor_meta_database
            .save()
            .map_err(|e| RustBreakWrappedError(e))
    }
}

type Stream<T> = Pin<
    Box<dyn futures_core::Stream<Item = std::result::Result<T, Status>> + Send + Sync + 'static>,
>;

#[tonic::async_trait]
impl TasksManager for TaskServer {
    type GetTasksStream = Stream<GetTaskStreamReply>;
    async fn get_tasks(
        &self,
        request: tonic::Request<GetTasksRequest>,
    ) -> Result<tonic::Response<Self::GetTasksStream>, tonic::Status> {
        let request = request.into_inner();
        let client_id = request.client_id.clone();
        // register the client and wait for new tasks to come, forward them
        // to the response
        let (sender, receiver) = mpsc::unbounded();
        self.register_executor(request.into(), sender);

        let tasks_sinks = self.tasks_sinks.clone();

        let response_stream = receiver.map(move |(task_payload, sender_to_commander)| {
            // for each new task, register the task and forward it to the executor stream
            let task_id = register_new_task(&tasks_sinks, sender_to_commander);
            info!(
                "Sending task {} - {} to {}",
                task_id, task_payload.payload, client_id
            );
            Ok(GetTaskStreamReply {
                task_id,
                task_payload: Some(task_payload),
            })
        });

        Ok(Response::new(
            Box::pin(response_stream) as Self::GetTasksStream
        ))
    }
    async fn task_execution(
        &self,
        request: tonic::Request<tonic::Streaming<TaskExecutionResult>>,
    ) -> Result<tonic::Response<TaskExecutionReply>, tonic::Status> {
        let task_id =
            String::from_utf8_lossy(request.metadata().get("task_id").unwrap().as_bytes())
                .into_owned();
        let mut request_stream = request.into_inner();
        if let Some(sender) = get_task_sink(&self.tasks_sinks, &task_id) {
            let mut sender = sender;
            while let Some(task_execution_stream) = request_stream.next().await {
                let task_execution_stream = task_execution_stream?;
                info!(
                    "Received task_execution_report {} - {}",
                    task_execution_stream.client_id, task_id
                );
                if let Err(_e) = sender.send(task_execution_stream).await {
                    warn!(
                        "Commander disconnected for task {}, task will be killed by executor.",
                        task_id
                    );
                    break;
                }
            }
            Ok(Response::new(TaskExecutionReply {}))
        } else {
            error!("Task id not found {}", task_id);
            Err(tonic::Status::new(Code::NotFound, "task_id not found"))
        }
    }
    type LaunchTaskStream = Stream<TaskExecutionResult>;

    async fn launch_task(
        &self,
        request: tonic::Request<LaunchTaskRequest>,
    ) -> Result<tonic::Response<Self::LaunchTaskStream>, tonic::Status> {
        let request = request.into_inner();
        let query = request.predicate;
        let payload = request.task_payload.unwrap();

        // this channel will be sent to the matching executors. the executors will then register it so
        // further task progression reporting could be sent o
        let (mut sender, receiver) = mpsc::unbounded();

        let mut senders = self
            .get_channels_to_matching_executors(&query)
            .map_err(|e| tonic::Status::new(Code::Internal, format!("Unexpected Error {}", e)))?;

        for (client_id, executor_sender) in senders.iter_mut() {
            if let Some(executor_sender) = executor_sender {
                if let Err(_) = executor_sender
                    .send((payload.clone(), sender.clone()))
                    .await
                {
                    error!("Executor {} disconnected!", client_id);
                    if let Err(_) = sender
                        .send(TaskExecutionResult {
                            task_id: random_task_id(),
                            client_id: client_id.clone(),
                            execution_result: Some(ExecutionResult::Disconnected(
                                ExecutorDisconnected {},
                            )),
                        })
                        .await
                    {
                        error!("Commander also disconnected!");
                    }
                }
            } else {
                if let Err(_) = sender
                    .send(TaskExecutionResult {
                        task_id: random_task_id(),
                        client_id: client_id.clone(),
                        execution_result: Some(ExecutionResult::Disconnected(
                            ExecutorDisconnected {},
                        )),
                    })
                    .await
                {
                    error!("Commander also disconnected!");
                }
            }
        }
        let response_stream = receiver.map(|task_execution| Ok(task_execution));
        Ok(Response::new(
            Box::pin(response_stream) as Self::LaunchTaskStream
        ))
    }
}

fn random_task_id() -> String {
    let id: u128 = rand::thread_rng().gen();
    format!("{:x}", id)
}
