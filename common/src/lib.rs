#![allow(unused_imports)]

#[macro_use]
extern crate async_stream;

#[macro_use]
extern crate log;

pub mod config;
pub mod executor_meta;
pub mod file_utils;
pub mod signed_payload;
pub mod task_server;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub const PROTOCOL_VERSION: &'static str = grpc_service::VERSION;

pub const QUERY_PARSER_VERSION: &'static str = query_parser::VERSION;
