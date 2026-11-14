//! MCP memory server. The agent stores and recalls durable statements.
//! HelixDB holds the knowledge graph in-process.

#![recursion_limit = "256"]

pub mod access;
pub mod config;
pub mod embed;
pub mod identity;
pub mod instructions;
pub mod mcp;
pub mod model;
pub mod rank;
pub mod secrets;
pub mod service;
pub mod store;

pub use config::Config;
pub use service::Service;
