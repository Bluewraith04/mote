//! The mote home and the cache of unpacked zips in it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use sha2::{Digest, Sha256};

use crate::archive::unpack;
use crate::package::shown;

/// The mote home: `$MOTE_HOME`, else `~/.mote`; none when neither is set.
pub fn default_home() -> Option<PathBuf> {
    home_of(std::env::var_os("MOTE_HOME"), std::env::var_os("HOME"))
}

fn home_of(mote_home: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    match mote_home.filter(|text| !text.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home.filter(|text| !text.is_empty()).map(|dir| PathBuf::from(dir).join(".mote")),
    }
}

/// The sha256 of the bytes as 64 hex digits.
pub fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The unpacked zips below `<home>/cache`.
#[derive(Clone, Debug)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn at(home: &Path) -> Cache {
        Cache { root: home.join("cache") }
    }

    /// The directory the zip with these bytes unpacks to, unpacking it first when it is not there.
    pub fn extract(&self, bytes: &[u8]) -> Result<PathBuf, String> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = self.root.join(hash(bytes));
        if dir.join("mote.toml").is_file() {
            return Ok(dir);
        }
        fs::create_dir_all(&self.root).map_err(|error| format!("cannot make {}: {error}", shown(&self.root)))?;
        let work = self.root.join(format!("tmp-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = fs::remove_dir_all(&work);
        let made = unpack(bytes, &work).and_then(|()| fs::rename(&work, &dir).or_else(|error| if dir.join("mote.toml").is_file() { Ok(()) } else { Err(error) }).map_err(|error| format!("cannot make {}: {error}", shown(&dir))));
        let _ = fs::remove_dir_all(&work);
        made.map(|()| dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::pack;
    use crate::testing::Scratch;

    #[test]
    fn cache_the_home_is_mote_home_else_dot_mote_below_home() {
        assert_eq!(home_of(Some("/m".into()), Some("/h".into())), Some(PathBuf::from("/m")));
        assert_eq!(home_of(None, Some("/h".into())), Some(PathBuf::from("/h/.mote")));
        assert_eq!(home_of(Some("".into()), Some("/h".into())), Some(PathBuf::from("/h/.mote")));
        assert_eq!(home_of(None, None), None);
    }

    #[test]
    fn cache_the_hash_is_sha256_in_hex() {
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn cache_unpacks_a_zip_once_into_a_directory_named_by_its_hash() {
        let scratch = Scratch::new();
        let packed = pack(&scratch.package("pane", "pane", &[], &[("lib.mote", "a")])).unwrap();
        let cache = Cache::at(&scratch.path("home"));
        let dir = cache.extract(&packed.bytes).unwrap();
        assert_eq!(dir, scratch.path("home/cache").join(hash(&packed.bytes)));
        assert_eq!(fs::read_to_string(dir.join("src/lib.mote")).unwrap(), "a");
        fs::write(dir.join("src/lib.mote"), "changed").unwrap();
        assert_eq!(cache.extract(&packed.bytes).unwrap(), dir);
        assert_eq!(fs::read_to_string(dir.join("src/lib.mote")).unwrap(), "changed");
        let leftovers: Vec<_> = fs::read_dir(scratch.path("home/cache")).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(leftovers, [hash(&packed.bytes)]);
    }

    #[test]
    fn cache_a_bad_zip_leaves_nothing_behind() {
        let scratch = Scratch::new();
        let cache = Cache::at(&scratch.path("home"));
        assert!(cache.extract(b"not a zip").unwrap_err().starts_with("not a zip file"));
        assert_eq!(fs::read_dir(scratch.path("home/cache")).unwrap().count(), 0);
    }
}
