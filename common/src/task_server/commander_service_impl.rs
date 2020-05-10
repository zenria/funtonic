use crate::executor_meta::ExecutorMeta;
use crate::task_server::{random_task_id, Stream, TaskServer};
use crate::{CLIENT_TOKEN_HEADER, PROTOCOL_VERSION};
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use grpc_service::grpc_protocol::admin_request::RequestType;
use grpc_service::grpc_protocol::admin_request_response::ResponseKind;
use grpc_service::grpc_protocol::commander_service_server::*;
use grpc_service::grpc_protocol::executor_service_server::*;
use grpc_service::grpc_protocol::launch_task_request::Task;
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
impl CommanderService for TaskServer {
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

        let mut senders = self.get_channels_to_matching_executors(&query)?;

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
        request: Request<SignedPayload>,
    ) -> Result<Response<AdminRequestResponse>, Status> {
        let token_name = self.verify_token(&request)?;

        let request = request.into_inner();
        info!("{} - {:?}", token_name, request);

        let request: AdminRequest = self.authorized_admin_keys.decode_payload(&request)?;

        match request
            .request_type
            .ok_or(Status::invalid_argument("Missing request type"))?
        {
            RequestType::ListConnectedExecutors(query) => {
                let connected_executors = self
                    .executors
                    .lock()
                    .map_err(|_| Status::internal("Unable to lock"))?
                    .iter()
                    .map(|(client_id, _)| client_id.clone())
                    .collect::<HashSet<_>>();

                Ok(Response::new(AdminRequestResponse {
                    response_kind: Some(ResponseKind::JsonResponse(
                        parse(&query)
                            .map_err(|parse_error| {
                                Status::invalid_argument(format!("Invalid query: {}", parse_error))
                            })
                            .and_then(|query| {
                                self.read_executor_meta_database(|data| {
                                    serde_json::to_string(
                                        &data
                                            .iter()
                                            .filter(|(client_id, meta)| {
                                                connected_executors.contains(*client_id)
                                                    && meta.qmatches(&query)
                                            })
                                            .collect::<BTreeMap<_, _>>(),
                                    )
                                })?
                                .map_err(|deser| {
                                    Status::internal(format!("An error occured: {}", deser))
                                })
                            })?,
                    )),
                }))
            }
            RequestType::ListKnownExecutors(query) => Ok(Response::new(AdminRequestResponse {
                response_kind: Some(ResponseKind::JsonResponse(
                    parse(&query)
                        .map_err(|parse_error| {
                            Status::invalid_argument(format!("Invalid query: {}", parse_error))
                        })
                        .and_then(|query| {
                            self.read_executor_meta_database(|data| {
                                serde_json::to_string(
                                    &data
                                        .iter()
                                        .filter(|(_, meta)| meta.qmatches(&query))
                                        .collect::<BTreeMap<_, _>>(),
                                )
                            })?
                            .map_err(|deser| {
                                Status::internal(format!("An error occured: {}", deser))
                            })
                        })?,
                )),
            })),
            RequestType::ListRunningTasks(_) => Ok(Response::new(AdminRequestResponse {
                response_kind: Some(ResponseKind::JsonResponse(
                    serde_json::to_string(
                        &self
                            .get_running_tasks()
                            .map_err(|e| Status::internal(e.to_string()))?,
                    )
                    .map_err(|deser| Status::internal(format!("An error occured: {}", deser)))?,
                )),
            })),
            RequestType::ListTokens(_) => Ok(Response::new(AdminRequestResponse {
                response_kind: Some(ResponseKind::JsonResponse(
                    serde_json::to_string(&self.get_token_names()).map_err(|deser| {
                        Status::internal(format!("An error occured: {}", deser))
                    })?,
                )),
            })),
            RequestType::DropExecutor(query) => {
                Ok(Response::new(AdminRequestResponse {
                    response_kind: Some(ResponseKind::JsonResponse(
                        serde_json::to_string(
                            &parse(&query)
                                .map_err(|parse_error| {
                                    Status::invalid_argument(format!(
                                        "Invalid query: {}",
                                        parse_error
                                    ))
                                })
                                .and_then(|query| {
                                    Ok(self.read_executor_meta_database(|data| {
                                        data.iter()
                                            .filter(|(_, meta)| meta.qmatches(&query))
                                            .map(|(client_id, _)| client_id.clone())
                                            .collect::<Vec<_>>()
                                    })?)
                                })
                                .and_then(|client_ids| {
                                    client_ids.into_iter().try_fold(
                                        BTreeMap::new(),
                                        |mut acc, client_id| {
                                            // remove from database
                                            let removed_from_known = self
                                                .write_executor_meta_database(|data| {
                                                    data.remove(&client_id).is_some()
                                                })?;
                                            // remove from connected executors
                                            let removed_from_connected = self
                                                .executors
                                                .lock()
                                                .map_err(|_| Status::internal("Unable to lock"))?
                                                .remove(&client_id)
                                                .is_some();
                                            acc.insert(
                                                client_id,
                                                AdminDroppedExecutorJsonResponse {
                                                    removed_from_connected,
                                                    removed_from_known,
                                                },
                                            );
                                            Ok(acc)
                                        },
                                    )
                                })?,
                        )
                        .map_err(|deser| {
                            Status::internal(format!("An error occured: {}", deser))
                        })?,
                    )),
                }))
            }
            RequestType::RevokeToken(_) => Err(Status::unimplemented("not implemented yet")),
            RequestType::GrantToken(_) => Err(Status::unimplemented("not implemented yet")),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AdminDroppedExecutorJsonResponse {
    pub removed_from_connected: bool,
    pub removed_from_known: bool,
}
