//! Packages: manifests, lockfiles, a SemVer resolver, project scaffolding and the bundler.
/// Bundling a program into a standalone executable.
pub mod bundler;
/// The `mote` project commands.
pub mod cli;
/// The global config and the registries a project uses.
pub mod config;
/// The fetched core library component.
pub mod core;
/// Installing dependencies.
pub mod installer;
/// The lockfile.
pub mod lockfile;
/// The `.mbc` bytecode container.
pub mod mbc;
pub mod mpk;
/// Git as a package source.
pub mod git;
/// The `mote.toml` manifest.
pub mod manifest;
/// Native libraries carried by packages.
pub mod native;
/// `mote publish`.
pub mod publish;
/// Finding the package `mote install` builds.
pub mod program;
/// The mote home and its cache of fetched packages.
pub mod store;
mod regdeps;
/// A package registry over HTTP.
pub mod registry;
/// Dependency resolution.
pub mod resolver;
/// Versions and version requirements.
pub mod semver;

pub use bundler::StandaloneBundler;
pub use cli::PackageManager;
pub use installer::PackageInstaller;
pub use lockfile::{LockedPackage, Lockfile};
pub use mbc::MbcFile;
pub use git::{Git, GitRef, Repo};
pub use mpk::MpkArchive;
pub use manifest::{DependencySpec, PackageManifest, PackageMeta, RunConfig};
pub use resolver::{AvailablePackage, DependencyResolver, ResolvedPackage};
pub use semver::{Version, VersionConstraint, VersionOp, VersionReq};
