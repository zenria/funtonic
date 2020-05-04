use crate::executor_meta::ExecutorMeta;
use crate::{CLIENT_TOKEN_HEADER, PROTOCOL_VERSION};
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use grpc_service::grpc_protocol::launch_task_response::TaskResponse;
use grpc_service::grpc_protocol::ExecuteCommand;
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
use tokio::time::Duration;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::{Code, Request, Response, Status, Streaming};

mod commander_service_impl;
mod executor_service_impl;

pub use commander_service_impl::AdminDroppedExecutorJsonResponse;

#[derive(Debug, Error)]
pub enum TaskServerError {
    #[error("Unable to get lock")]
    LockError,
    #[error("Rustbreak database error {0}")]
    DatabaseError(rustbreak::RustbreakError),
}

impl From<TaskServerError> for Status {
    fn from(e: TaskServerError) -> Self {
        Status::internal(e.to_string())
    }
}

type ExecutorMetaDatabase = HashMap<String, ExecutorMeta>;

#[derive(Clone)]
pub struct TaskServer {
    /// executors by id: when a task must be submited to an executor,
    /// a Sender is sent to each matching executor
    executors: Arc<
        Mutex<
            HashMap<
                String,
                mpsc::UnboundedSender<(ExecuteCommand, mpsc::UnboundedSender<TaskResponse>)>,
            >,
        >,
    >,

    /// by task id, sinks where executors reports task execution
    tasks_sinks: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TaskResponse>>>>,

    executor_meta_database: Arc<FileDatabase<ExecutorMetaDatabase, Yaml>>,

    /// map<token, name> the name is used for logging purpose
    authorized_client_tokens: Arc<BTreeMap<String, String>>,
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
            .map_err(|e| TaskServerError::DatabaseError(e))?;
        db.load().map_err(|e| TaskServerError::DatabaseError(e))?;
        Ok(TaskServer {
            executors: Arc::new(Mutex::new(HashMap::new())),
            tasks_sinks: Arc::new(Mutex::new(HashMap::new())),
            executor_meta_database: Arc::new(db),
            authorized_client_tokens: Arc::new(authorized_client_tokens),
        })
    }

    pub fn start_heartbeat(&self) {
        tokio::spawn(heartbeat(self.executors.clone()));
    }

    fn get_channels_to_matching_executors(
        &self,
        query: &Query,
    ) -> Result<
        Vec<(
            String,
            Option<mpsc::UnboundedSender<(ExecuteCommand, mpsc::UnboundedSender<TaskResponse>)>>,
        )>,
        TaskServerError,
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
            .map_err(|e| TaskServerError::DatabaseError(e))?;

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
    ) -> Result<(), TaskServerError> {
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
            .map_err(|e| TaskServerError::DatabaseError(e))?;
        self.executor_meta_database
            .save()
            .map_err(|e| TaskServerError::DatabaseError(e))
    }
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

    fn get_token_names(&self) -> Vec<&str> {
        self.authorized_client_tokens
            .iter()
            .map(|(_, name)| name.as_str())
            .collect()
    }

    fn get_running_tasks(&self) -> Result<Vec<String>, TaskServerError> {
        Ok(self
            .tasks_sinks
            .lock()
            .map_err(|_e| TaskServerError::LockError)?
            .iter()
            .map(|(task_id, _)| task_id.clone())
            .collect())
    }

    fn read_executor_meta_database<F: Fn(&ExecutorMetaDatabase) -> R, R>(
        &self,
        read_function: F,
    ) -> Result<R, TaskServerError> {
        self.executor_meta_database
            .read(read_function)
            .map_err(|e| TaskServerError::DatabaseError(e))
    }

    fn write_executor_meta_database<F: FnOnce(&mut ExecutorMetaDatabase) -> R, R>(
        &self,
        write_function: F,
    ) -> Result<R, TaskServerError> {
        self.executor_meta_database
            .write(write_function)
            .map_err(|e| TaskServerError::DatabaseError(e))
    }
}

async fn heartbeat(
    _executors: Arc<
        Mutex<
            HashMap<
                String,
                mpsc::UnboundedSender<(ExecuteCommand, mpsc::UnboundedSender<TaskResponse>)>,
            >,
        >,
    >,
) {
    loop {
        tokio::time::delay_for(Duration::from_secs(5)).await;
        debug!("Checking connected executor health");
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

type Stream<T> =
    Pin<Box<dyn futures::Stream<Item = std::result::Result<T, Status>> + Send + Sync + 'static>>;

fn random_task_id() -> String {
    let id: u128 = rand::thread_rng().gen();
    format!("{:x}", id)
}
