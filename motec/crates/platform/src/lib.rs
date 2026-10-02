//! `Platform` implementations: the real machine, a deterministic fake, and their shared conformance suite.

pub mod conformance;
mod dynlib;
mod fake;
mod fake_net;
mod http;
pub mod memory;
mod net;
mod reactor;
mod sql;
mod system;
pub mod tls;

pub use fake::FakePlatform;
pub use system::SystemPlatform;
