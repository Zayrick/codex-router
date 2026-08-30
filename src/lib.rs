//! Standalone Codex protocol router.
//!
//! Protocol and application modules are runtime-neutral. Native HTTP, file
//! configuration, scheduling, and upstream I/O live in [`server`].

pub mod application;
pub mod auth;
pub mod core;
pub mod http;
pub mod protocol;
pub mod server;
pub mod upstream;
