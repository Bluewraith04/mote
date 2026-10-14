//! Multi-file compilation: import resolution, the dependency graph, name mangling and visibility.
/// Where the core libraries live.
pub mod core;
/// The multi-file compile driver.
pub mod driver;
pub mod embedded;
/// The module dependency graph.
pub mod graph;
pub(crate) mod modifier_host;
/// Canonical module identifiers.
pub mod path;
/// Import resolution and parsing of modules.
pub mod resolver;
pub mod rewrite;
/// Stage 0: runs a `@modifier` function at compile time.
pub mod stage0;
pub mod symbols;
/// Export tables and visibility checks.
pub mod visibility;

pub use driver::MultiFileCompiler;
pub use graph::{DependencyGraph, ModuleNode};
pub use path::CanonicalModuleId;
pub use resolver::ModuleResolver;
pub use symbols::{module_prefixes, AliasTarget, ModuleScope};
pub use visibility::{ModuleExports, VisibilityChecker};
