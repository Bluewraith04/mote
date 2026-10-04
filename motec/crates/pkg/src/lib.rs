//! Packages: `mote.toml` and `mote.lock`, a SemVer resolver, project scaffolding and the standalone bundler.
/// Bundling a program into a standalone executable.
pub mod bundler;
/// The `mote` project commands.
pub mod cli;
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
