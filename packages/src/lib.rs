//! Reads manifests, packs and unpacks zip files, and loads the packages of a build.

mod archive;
mod bundle;
mod cache;
mod graph;
mod libraries;
mod manifest;
mod native;
mod package;
mod standard;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod toml;

pub use archive::{pack, unpack, Packed, MAX_BYTES, MAX_ENTRIES};
pub use bundle::unpack_natives;
pub use cache::{default_home, hash, Cache};
pub use graph::{build, Graph, PackageModules};
pub use libraries::{core_names, core_packages, embedded_core, packages_in};
pub use manifest::{is_name, Dependency, Manifest, PackageError, Source, Version, DEFAULT_ENTRY};
pub use native::{archive_file, host_triple, library_file, natives, Library, Location, Natives};
pub use package::{is_reserved, shown, Package, Packages, SourceFile, RESERVED};
pub use standard::{embedded, standard};
