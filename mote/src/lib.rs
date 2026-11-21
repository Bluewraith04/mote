//! Runs the compile pipeline from source to running program.

pub mod cli;
pub mod executable;
mod deps;
mod init;
mod pipeline;
mod requirements;
mod run;

pub use cli::{execute, execute_at, execute_live, Outcome};
pub use pipeline::{toolchain, Failure, Options, Pipeline, Target};
pub use run::{run_main, run_main_fed, run_main_with, run_tests, run_tests_with, Ended, Feed, Finished, MainRun, Settings, TestLine, TestRun, Verdict};
