//! Multi-file compilation: import resolution, the dependency graph, name mangling and visibility.
/// The multi-file compile driver.
pub mod driver;
pub mod embedded;
/// The module dependency graph.
pub mod graph;
/// Canonical module identifiers.
pub mod path;
/// Import resolution and parsing of modules.
pub mod resolver;
pub mod rewrite;
pub mod symbols;
/// Export tables and visibility checks.
pub mod visibility;

pub use driver::MultiFileCompiler;
pub use graph::{DependencyGraph, ModuleNode};
pub use path::CanonicalModuleId;
pub use resolver::ModuleResolver;
pub use symbols::{module_prefixes, AliasTarget, ModuleScope};
pub use visibility::{ModuleExports, VisibilityChecker};
