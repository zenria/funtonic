#![allow(unused_imports)]

#[macro_use]
extern crate async_stream;

#[macro_use]
extern crate log;

pub mod config;
pub mod crypto;
pub mod executor_meta;
pub mod file_utils;
pub mod path_builder;
pub mod task_server;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub const PROTOCOL_VERSION: &'static str = grpc_service::VERSION;

pub const QUERY_PARSER_VERSION: &'static str = query_parser::VERSION;

/// Maximum size of a decoded gRPC message (tonic defaults to 4MiB), large enough to upload files.
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

pub use grpc_service::prost;
pub use grpc_service::tonic;
pub use tokio;

pub mod error;

pub use data_encoding;
