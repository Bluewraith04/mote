//! The natives a release build embeds, unpacked under the home the first time a program uses their library.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use flate2::read::GzDecoder;

use crate::manifest::PackageError;
use crate::native::host_triple;
use crate::package::{shown, Packages};
use crate::standard::embedded::{self, EmbeddedNative};

fn failure(path: &Path, error: &io::Error) -> PackageError {
    PackageError { file: shown(path), line: 0, key: String::new(), message: format!("cannot unpack the embedded native: {error}") }
}

/// Writes `bytes` at `path` through a temporary file and a rename, so a process running at the same time never sees half of it.
fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("native");
    let temporary = path.with_file_name(format!(".{name}.{}", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

/// Puts the files of `natives` that belong to `libraries` in `<home>/core/<library>/native/<triple>/`, except those already there at this version.
fn unpack_files(natives: &[EmbeddedNative], libraries: &[&str], home: &Path) -> Result<(), PackageError> {
    for (library, file, version, packed) in natives {
        if !libraries.contains(library) {
            continue;
        }
        let dir = home.join("core").join(library).join("native").join(host_triple());
        let (path, stamp) = (dir.join(file), dir.join(format!(".{file}.version")));
        if path.is_file() && fs::read_to_string(&stamp).is_ok_and(|text| text.trim() == version.to_string()) {
            continue;
        }
        fs::create_dir_all(&dir).map_err(|error| failure(&dir, &error))?;
        let mut bytes = Vec::new();
        GzDecoder::new(*packed).read_to_end(&mut bytes).map_err(|error| failure(&path, &error))?;
        replace(&path, &bytes).map_err(|error| failure(&path, &error))?;
        replace(&stamp, version.to_string().as_bytes()).map_err(|error| failure(&stamp, &error))?;
    }
    Ok(())
}

/// Unpacks the embedded natives of each embedded library the build uses; a build with `--core` or without a home uses none.
pub fn unpack_natives(packages: &Packages, home: &Path) -> Result<(), PackageError> {
    let used: Vec<&str> = packages.list.iter().filter(|package| package.dir == home.join("core").join(&package.manifest.name)).map(|package| package.manifest.name.as_str()).collect();
    unpack_files(embedded::NATIVES, &used, home)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;

    use super::*;
    use crate::testing::Scratch;

    fn packed(bytes: &[u8]) -> &'static [u8] {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        Box::leak(encoder.finish().unwrap().into_boxed_slice())
    }

    fn files_at(home: &Path, library: &str) -> std::path::PathBuf {
        home.join("core").join(library).join("native").join(host_triple())
    }

    #[test]
    fn core_bundle_a_used_library_gets_its_files_under_the_home() {
        let scratch = Scratch::new();
        let natives = [("net", "libnet.so", 1, packed(b"shared")), ("net", "libnet.a", 1, packed(b"archive")), ("sqlite", "libsqlite.so", 1, packed(b"other"))];
        unpack_files(&natives, &["net"], &scratch.dir).unwrap();
        assert_eq!(fs::read(files_at(&scratch.dir, "net").join("libnet.so")).unwrap(), b"shared");
        assert_eq!(fs::read(files_at(&scratch.dir, "net").join("libnet.a")).unwrap(), b"archive");
        assert!(!files_at(&scratch.dir, "sqlite").exists());
    }

    #[test]
    fn core_bundle_a_file_at_the_current_version_is_left_alone_and_an_older_one_is_replaced() {
        let scratch = Scratch::new();
        unpack_files(&[("net", "libnet.so", 1, packed(b"one"))], &["net"], &scratch.dir).unwrap();
        let path = files_at(&scratch.dir, "net").join("libnet.so");
        fs::write(&path, b"changed").unwrap();
        unpack_files(&[("net", "libnet.so", 1, packed(b"one"))], &["net"], &scratch.dir).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"changed");
        unpack_files(&[("net", "libnet.so", 2, packed(b"two"))], &["net"], &scratch.dir).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn core_bundle_a_missing_file_is_written_again_at_the_same_version() {
        let scratch = Scratch::new();
        let natives = [("net", "libnet.so", 1, packed(b"one"))];
        unpack_files(&natives, &["net"], &scratch.dir).unwrap();
        let path = files_at(&scratch.dir, "net").join("libnet.so");
        fs::remove_file(&path).unwrap();
        unpack_files(&natives, &["net"], &scratch.dir).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
    }
}
