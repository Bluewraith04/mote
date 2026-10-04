//! `Platform` implementations: the real machine, a deterministic fake, and their shared conformance suite.

pub mod conformance;
mod dynlib;
mod ext;
mod fake;
mod fake_net;
pub mod memory;
mod net;
mod reactor;
mod system;
pub mod tls;

pub use fake::FakePlatform;
pub use system::SystemPlatform;
