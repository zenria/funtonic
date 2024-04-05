use crate::executor_meta::ExecutorMeta;
use crate::tonic;
use crate::PROTOCOL_VERSION;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use grpc_service::grpc_protocol::launch_task_response::TaskResponse;
use grpc_service::grpc_protocol::{ExecuteCommand, GetTasksRequest};
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

use crate::crypto::keystore::{
    file_keystore, memory_keystore, FileKeyStoreBackend, KeyStore, KeyStoreError,
    MemoryKeyStoreBackend,
};
use crate::file_utils::path_concat2;
pub use commander_service_impl::{
    AdminDroppedExecutorJsonResponse, AdminListExecutorKeysJsonResponse,
};
use grpc_service::payload::SignedPayload;

#[derive(Debug, Error)]
pub enum TaskServerError {
    #[error("Unable to get lock")]
    LockError,
    #[error("Rustbreak database error {0}")]
    DatabaseError(#[from] rustbreak::RustbreakError),
    #[error("Internal key store error {0}")]
    KeyStoreError(#[from] KeyStoreError),
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
                mpsc::UnboundedSender<(SignedPayload, mpsc::UnboundedSender<TaskResponse>)>,
            >,
        >,
    >,

    /// by task id, sinks where executors reports task execution
    tasks_sinks: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TaskResponse>>>>,

    executor_meta_database: Arc<FileDatabase<ExecutorMetaDatabase, Yaml>>,

    authorized_keys: Arc<KeyStore<MemoryKeyStoreBackend>>,

    authorized_admin_keys: Arc<KeyStore<MemoryKeyStoreBackend>>,

    trusted_executor_keystore: Arc<KeyStore<FileKeyStoreBackend>>,

    unapproved_executor_keystore: Arc<KeyStore<FileKeyStoreBackend>>,
}

impl TaskServer {
    pub fn new<P: AsRef<Path>>(
        database_dir: P,
        authorized_keys: &BTreeMap<String, String>,
        admin_authorized_keys: &BTreeMap<String, String>,
    ) -> Result<Self, anyhow::Error> {
        let database_path = path_concat2(&database_dir, "known_executors.yml");
        if !database_path.exists() {
            let mut empty = File::create(&database_path)?;
            empty.write("---\n{}".as_bytes())?;
        }

        let db = FileDatabase::load_from_path_or_default(database_path)?;
        db.load()?;
        Ok(TaskServer {
            executors: Arc::new(Mutex::new(HashMap::new())),
            tasks_sinks: Arc::new(Mutex::new(HashMap::new())),
            executor_meta_database: Arc::new(db),
            authorized_keys: Arc::new(memory_keystore().init_from_map(authorized_keys)?),
            authorized_admin_keys: Arc::new(
                memory_keystore().init_from_map(admin_authorized_keys)?,
            ),
            trusted_executor_keystore: Arc::new(file_keystore(path_concat2(
                &database_dir,
                "trusted_executors_keys.yml",
            ))?),
            unapproved_executor_keystore: Arc::new(file_keystore(path_concat2(
                &database_dir,
                "unapproved_executors_keys.yml",
            ))?),
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
            Option<mpsc::UnboundedSender<(SignedPayload, mpsc::UnboundedSender<TaskResponse>)>>,
        )>,
        TaskServerError,
    > {
        let client_ids: Vec<String> = self.executor_meta_database.read(|executors| {
            executors
                .iter()
                .filter(|(_client_id, meta)| meta.qmatches(query).matches())
                .map(|(client_id, _meta)| client_id.clone())
                .collect()
        })?;

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
        request: &GetTasksRequest,
        sender_to_get_task_response: mpsc::UnboundedSender<(
            SignedPayload,
            mpsc::UnboundedSender<TaskResponse>,
        )>,
    ) -> Result<(), TaskServerError> {
        let executor_meta: ExecutorMeta = request.into();

        self.executors.lock().unwrap().insert(
            executor_meta.client_id().to_string(),
            sender_to_get_task_response,
        );

        self.executor_meta_database.write(move |executors| {
            info!(
                "Registered {}",
                serde_yaml::to_string(&executor_meta).unwrap_or("???".to_string())
            );
            executors.insert(executor_meta.client_id().to_string(), executor_meta);
        })?;

        for public_key in &request.authorized_keys {
            self.authorized_keys
                .register_key(&public_key.key_id, public_key.key_bytes.clone())?;
        }

        Ok(self.executor_meta_database.save()?)
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
        Ok(self.executor_meta_database.read(read_function)?)
    }

    fn write_executor_meta_database<F: FnOnce(&mut ExecutorMetaDatabase) -> R, R>(
        &self,
        write_function: F,
    ) -> Result<R, TaskServerError> {
        Ok(self.executor_meta_database.write(write_function)?)
    }

    /// Handle executor public key.
    ///
    /// If the key is known and authorized, does nothing.
    ///
    /// If the key is not known for this client_id, register it in unapproved_executor_keystore
    fn handle_executor_key(&self, client_id: &str, key_bytes: &[u8]) -> Result<(), KeyStoreError> {
        if !self
            .trusted_executor_keystore
            .has_key(client_id, key_bytes)?
            && !self
                .unapproved_executor_keystore
                .has_key(client_id, key_bytes)?
        {
            self.unapproved_executor_keystore
                .register_key(client_id, key_bytes.to_vec())
        } else {
            Ok(())
        }
    }

    fn approve_executor_key(&self, client_id: &str) -> Result<(), KeyStoreError> {
        self.trusted_executor_keystore.register_key(
            client_id,
            self.unapproved_executor_keystore.remove_key(client_id)?,
        )
    }

    fn list_trusted_executor_keys(&self) -> Result<BTreeMap<String, String>, KeyStoreError> {
        self.trusted_executor_keystore.list_all()
    }

    fn list_unapproved_executor_keys(&self) -> Result<BTreeMap<String, String>, KeyStoreError> {
        self.unapproved_executor_keystore.list_all()
    }
}

async fn heartbeat(
    _executors: Arc<
        Mutex<
            HashMap<
                String,
                mpsc::UnboundedSender<(SignedPayload, mpsc::UnboundedSender<TaskResponse>)>,
            >,
        >,
    >,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
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
