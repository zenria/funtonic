use crate::generated::tasks::server::*;
use crate::generated::tasks::task_execution_stream::ExecutionResult;
use crate::generated::tasks::*;
use futures_channel::mpsc;
use futures_sink::Sink;
use futures_util::pin_mut;
use futures_util::{FutureExt, SinkExt, StreamExt};
use rand::Rng;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status, Streaming};

pub struct TaskServer {
    /// executors by id: when a task must be submited to an executor,
    /// a Sender is sent to each matching executor
    executors: Mutex<
        HashMap<
            String,
            mpsc::UnboundedSender<(TaskPayload, mpsc::UnboundedSender<TaskExecutionStream>)>,
        >,
    >,

    /// by task id, sinks where executors reports task execution
    tasks_sinks: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionStream>>>>,
}

impl TaskServer {
    pub fn new() -> Self {
        TaskServer {
            executors: Mutex::new(HashMap::new()),
            tasks_sinks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn register_new_task(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionStream>>>,
    sender_to_commander: mpsc::UnboundedSender<TaskExecutionStream>,
) -> String {
    let task_id = random_task_id();
    tasks_sinks
        .lock()
        .unwrap()
        .insert(task_id.clone(), sender_to_commander);
    task_id
}

fn get_task_sink(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionStream>>>,
    task_id: &str,
) -> Option<mpsc::UnboundedSender<TaskExecutionStream>> {
    tasks_sinks
        .lock()
        .unwrap()
        .get(task_id)
        .map(|sender| sender.clone())
}

fn task_completed(
    tasks_sinks: &Mutex<HashMap<String, mpsc::UnboundedSender<TaskExecutionStream>>>,
    task_id: &str,
) {
    tasks_sinks.lock().unwrap().remove(task_id);
}

impl TaskServer {
    fn get_channels_to_matching_executors(
        &self,
        _query: &str,
    ) -> Vec<mpsc::UnboundedSender<(TaskPayload, mpsc::UnboundedSender<TaskExecutionStream>)>> {
        // this code needs to be done in a separate block because aht executors variable is not Send,
        // thus, if it resides in the stack it will fail the whole Future stuff
        //
        let mut executor_senders = self.executors.lock().unwrap();
        // find matching senders, clone them
        executor_senders
            .iter_mut()
            .map(|(client_id, executor_sender)| executor_sender.clone())
            .collect()
    }

    fn register_executor(
        &self,
        client_id: &str,
        sender_to_get_task_response: mpsc::UnboundedSender<(
            TaskPayload,
            mpsc::UnboundedSender<TaskExecutionStream>,
        )>,
    ) {
        self.executors
            .lock()
            .unwrap()
            .insert(client_id.to_string(), sender_to_get_task_response);
    }
}

type Stream<T> =
    Pin<Box<dyn futures_core::Stream<Item = std::result::Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl TasksManager for TaskServer {
    async fn register_client(
        &self,
        request: tonic::Request<RegisterClientRequest>,
    ) -> Result<tonic::Response<RegisterClientReply>, tonic::Status> {
        Err(tonic::Status::unimplemented("Not yet implemented"))
    }
    type GetTasksStream = Stream<GetTaskStreamReply>;
    async fn get_tasks(
        &self,
        request: tonic::Request<GetTasksRequest>,
    ) -> Result<tonic::Response<Self::GetTasksStream>, tonic::Status> {
        let request = request.into_inner();

        // register the client and wait for new tasks to come, forward them
        // to the response
        let (sender, mut receiver) = mpsc::unbounded();
        self.register_executor(&request.client_id, sender);

        let tasks_sinks = self.tasks_sinks.clone();

        let response_stream = receiver.map(move |(task_payload, sender_to_commander)| {
            // for each new task, register the task and forward it to the executor stream
            let task_id = register_new_task(&tasks_sinks, sender_to_commander);
            println!(
                "Sending task {} - {} to {}",
                task_id, task_payload.payload, request.client_id
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
        request: tonic::Request<tonic::Streaming<TaskExecutionStream>>,
    ) -> Result<tonic::Response<TaskExecutionReply>, tonic::Status> {
        let mut request_stream = request.into_inner();

        let tasks_sinks = self.tasks_sinks.clone();
        while let Some(task_execution_stream) = request_stream.next().await {
            let task_execution_stream = task_execution_stream?;
            println!(
                "Received task_execution_report {} - {}",
                task_execution_stream.client_id, task_execution_stream.task_id
            );
            let is_task_completed = match task_execution_stream.execution_result.as_ref().unwrap() {
                ExecutionResult::TaskCompleted(_) => true,
                _ => false,
            };
            if let Some(sender) = get_task_sink(&tasks_sinks, &task_execution_stream.task_id) {
                let mut sender = sender;
                if is_task_completed {
                    // unregister the sink
                    task_completed(&tasks_sinks, &task_execution_stream.task_id);
                }
                sender.send(task_execution_stream).await;
            } else {
                eprintln!("Task id not found {}", task_execution_stream.task_id);
            }
        }

        Ok(Response::new(TaskExecutionReply {}))
    }
    type LaunchTaskStream = Stream<TaskExecutionStream>;

    async fn launch_task(
        &self,
        request: tonic::Request<LaunchTaskRequest>,
    ) -> Result<tonic::Response<Self::LaunchTaskStream>, tonic::Status> {
        let request = request.into_inner();
        let query = request.predicate;
        let payload = request.task_payload.unwrap();

        // this channel will be sent to the matching executors. the executors will then register it so
        // further task progression reporting could be sent o
        let (sender, receiver) = mpsc::unbounded();

        let mut senders = self.get_channels_to_matching_executors(&query);

        for executor_sender in senders.iter_mut() {
            executor_sender
                .send((payload.clone(), sender.clone()))
                .await;
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
