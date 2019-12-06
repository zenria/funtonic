#![allow(unused_imports)]

#[macro_use]
extern crate async_stream;

#[macro_use]
extern crate log;

pub mod config;
pub mod exec;
pub mod executor_meta;
pub mod file_utils;
pub mod task_server;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub const PROTOCOL_VERSION: &'static str = grpc_service::VERSION;

pub const CLIENT_TOKEN_HEADER: &'static str = "client_token";
