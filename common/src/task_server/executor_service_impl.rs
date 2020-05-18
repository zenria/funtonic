use super::Stream;
use crate::executor_meta::ExecutorMeta;
use crate::task_server::{get_task_sink, register_new_task, TaskServer};
use crate::PROTOCOL_VERSION;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use grpc_service::grpc_protocol::admin_request::RequestType;
use grpc_service::grpc_protocol::admin_request_response::ResponseKind;
use grpc_service::grpc_protocol::commander_service_server::*;
use grpc_service::grpc_protocol::executor_service_server::*;
use grpc_service::grpc_protocol::launch_task_response::TaskResponse;
use grpc_service::grpc_protocol::task_execution_result::ExecutionResult;
use grpc_service::grpc_protocol::*;
use grpc_service::payload::SignedPayload;
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

#[tonic::async_trait]
impl ExecutorService for TaskServer {
    type GetTasksStream = Stream<GetTaskStreamReply>;
    async fn get_tasks(
        &self,
        request: tonic::Request<RegisterExecutorRequest>,
    ) -> Result<tonic::Response<Self::GetTasksStream>, tonic::Status> {
        let metadata = request.metadata();
        let request = request.get_ref();

        // check the public key of the executor
        self.handle_executor_key(&request.client_id, &request.public_key)?;

        // decode the payload
        let request: GetTasksRequest = self.trusted_executor_keystore.decode_payload(
            request
                .get_tasks_request
                .as_ref()
                .ok_or(Status::invalid_argument("Missing getTasksRequest"))?,
        )?;

        // Strict protocol version check
        if request.client_protocol_version != PROTOCOL_VERSION {
            warn!(
                "{} has protocol version {}, but expecting protocol version {}",
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

        // TODO check executor key before registering it
        //self.handle_executor_key(&client_id, request.)

        let client_id = request.client_id.clone();
        info!("{} connected with meta {:?}", client_id, metadata);
        // register the client and wait for new tasks to come, forward them
        // to the response
        let (sender, receiver) = mpsc::unbounded();
        if let Err(e) = self.register_executor((&request).into(), sender) {
            error!("Unable to register executor {}", e);
            Err(e)?;
        }

        let tasks_sinks = self.tasks_sinks.clone();

        let response_stream = receiver.map(move |(payload, sender_to_commander)| {
            // for each new task, register the task and forward it to the executor stream
            let task_id = register_new_task(&tasks_sinks, sender_to_commander);
            info!("Sending task {} - {:?} to {}", task_id, payload, client_id);
            Ok(GetTaskStreamReply {
                task_id,
                payload: Some(payload),
            })
        });

        Ok(Response::new(
            Box::pin(response_stream) as Self::GetTasksStream
        ))
    }
    async fn task_execution(
        &self,
        request: tonic::Request<tonic::Streaming<SignedPayload>>,
    ) -> Result<tonic::Response<Empty>, tonic::Status> {
        let task_id =
            String::from_utf8_lossy(request.metadata().get("task_id").unwrap().as_bytes())
                .into_owned();

        let mut request_stream = request.into_inner();
        if let Some(sender) = get_task_sink(&self.tasks_sinks, &task_id) {
            let mut sender = sender;
            while let Some(task_execution_stream) = request_stream.next().await {
                let signed_payload = task_execution_stream?;
                let task_execution_stream: TaskExecutionResult = self
                    .trusted_executor_keystore
                    .decode_payload(&signed_payload)?;

                debug!(
                    "Received task_execution_report {} - {}",
                    task_execution_stream.client_id, task_id
                );
                if let Some(execution_result) = &task_execution_stream.execution_result {
                    if let ExecutionResult::TaskRejected(reason) = execution_result {
                        info!(
                            "Task {} rejected ({}) on {}",
                            task_id, reason, task_execution_stream.client_id,
                        );
                    }
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
}
