//! proxyhand: an HTTP(S) interception proxy with a built-in flow inspector.
//!
//! The binary (`main.rs`) is a thin CLI around this library: it parses arguments, builds the
//! pieces below and serves until interrupted. Integration tests drive the same pieces in-process.

pub mod api;
pub mod ca;
pub mod har;
pub mod i18n;
pub mod proxy;
pub mod sse;
pub mod store;
pub mod tls;
pub mod util;
pub mod ws;

/// The crate version, also reported by `--version` and the `/api/state` endpoint.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
