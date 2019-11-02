#![allow(unused_imports)]

#[macro_use]
extern crate async_stream;

#[macro_use]
extern crate log;

pub mod config;
pub mod exec;
pub mod file_utils;
pub mod generated;
pub mod task_server;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");
