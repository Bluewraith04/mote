//! The commands that make and use zip packages: `package`, `add`, `remove`, and finding what `install` builds.

use std::fs;
use std::path::{Component, Path, PathBuf};

use mote_packages::{default_home, is_name, is_reserved, pack, shown, Cache, Manifest, Source};

/// Writes `target/<name>-<version>.zip` of the package in `dir` and says what it holds.
pub fn package(dir: &Path) -> Result<String, String> {
    let packed = pack(dir).map_err(|error| error.to_string())?;
    let target = dir.join("target");
    fs::create_dir_all(&target).map_err(|error| format!("cannot make {}: {error}", shown(&target)))?;
    let file = target.join(&packed.file);
    fs::write(&file, &packed.bytes).map_err(|error| format!("cannot write {}: {error}", shown(&file)))?;
    Ok(format!("wrote {} ({} files, {} bytes)\n", shown(&file), packed.files, packed.bytes.len()))
}

/// The home to unpack zips in.
fn home_of(home: Option<&Path>) -> Result<PathBuf, String> {
    home.map(Path::to_path_buf).or_else(default_home).ok_or_else(|| "no mote home: set MOTE_HOME or HOME".to_string())
}

/// The directory a zip file unpacks to in the cache, and the manifest of the package in it.
pub fn unzipped(zip: &Path, home: Option<&Path>) -> Result<(PathBuf, Manifest), String> {
    let bytes = fs::read(zip).map_err(|error| format!("cannot read {}: {error}", shown(zip)))?;
    let dir = Cache::at(&home_of(home)?).extract(&bytes).map_err(|message| format!("{}: {message}", shown(zip)))?;
    let manifest = Manifest::read(&dir.join("mote.toml")).map_err(|error| error.to_string())?;
    Ok((dir, manifest))
}

/// `to` as a path from the directory `from`, both real.
fn relative(from: &Path, to: &Path) -> PathBuf {
    let (from, to): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut path: PathBuf = from[shared..].iter().map(|_| Component::ParentDir).collect();
    path.extend(&to[shared..]);
    path
}

/// The manifest text with the dependency `name` set to the table `entry`: its line replaced, else added at the end of `[dependencies]`.
fn with_entry(text: &str, manifest: &Manifest, name: &str, entry: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    let line = format!("{name} = {entry}");
    if let Some(found) = manifest.dependencies.iter().find(|dep| dep.name == name) {
        lines[found.line as usize - 1] = &line;
    } else if let Some(header) = lines.iter().position(|text| text.trim() == "[dependencies]") {
        let mut end = lines[header + 1..].iter().position(|text| text.trim_start().starts_with('[')).map_or(lines.len(), |next| header + 1 + next);
        while end > header + 1 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        lines.insert(end, &line);
    } else {
        if lines.last().is_some_and(|last| !last.trim().is_empty()) {
            lines.push("");
        }
        lines.push("[dependencies]");
        lines.push(&line);
    }
    lines.join("\n") + "\n"
}

/// Adds the zip file or the directory `what` (relative to `cwd`) as a dependency of the package in `project`.
pub fn add(project: &Path, cwd: &Path, what: &str, home: Option<&Path>) -> Result<String, String> {
    let given = cwd.join(what);
    let root_file = project.join("mote.toml");
    let text = fs::read_to_string(&root_file).map_err(|error| format!("cannot read {}: {error}", shown(&root_file)))?;
    let root = Manifest::parse(&shown(&root_file), &text).map_err(|error| error.to_string())?;
    let real_project = fs::canonicalize(project).map_err(|error| format!("cannot read {}: {error}", shown(project)))?;
    let (manifest, entry, said) = if given.is_dir() {
        let real = fs::canonicalize(&given).map_err(|error| format!("cannot read {}: {error}", shown(&given)))?;
        let manifest = Manifest::read(&real.join("mote.toml")).map_err(|error| error.to_string())?;
        let path = relative(&real_project, &real).display().to_string();
        let path = if path.is_empty() { ".".to_string() } else { path };
        (manifest, format!("{{ path = \"{path}\" }}"), path)
    } else if given.is_file() && given.extension().is_some_and(|extension| extension == "zip") {
        let (_, manifest) = unzipped(&given, home)?;
        let real = fs::canonicalize(&given).map_err(|error| format!("cannot read {}: {error}", shown(&given)))?;
        let kept = if real.starts_with(&real_project) {
            relative(&real_project, &real)
        } else {
            let file = real.file_name().ok_or_else(|| format!("{} has no name", shown(&given)))?;
            let inside = PathBuf::from("deps").join(file);
            let dest = project.join(&inside);
            fs::create_dir_all(project.join("deps")).map_err(|error| format!("cannot make {}: {error}", shown(&project.join("deps"))))?;
            fs::copy(&real, &dest).map_err(|error| format!("cannot write {}: {error}", shown(&dest)))?;
            inside
        };
        let path = kept.to_str().ok_or_else(|| "the path is not UTF-8".to_string())?.to_string();
        (manifest, format!("{{ zip = \"{path}\" }}"), path)
    } else {
        return Err(format!("{} is not a zip file or a package directory", shown(&given)));
    };
    let name = manifest.name.clone();
    if !is_name(&name) || is_reserved(&name) {
        return Err(format!("`{name}` is not a package name"));
    }
    if name == root.name {
        return Err(format!("`{name}` is this package"));
    }
    fs::write(&root_file, with_entry(&text, &root, &name, &entry)).map_err(|error| format!("cannot write {}: {error}", shown(&root_file)))?;
    let mut out = format!("added {name} {} ({said})\n", manifest.version);
    for dep in &manifest.dependencies {
        if dep.name != name && !root.dependencies.iter().any(|have| have.name == dep.name) {
            out.push_str(&format!("{name} needs {}; add it\n", dep.name));
        }
    }
    Ok(out)
}

/// Removes the dependency `name` from the manifest of the package in `project`; a zip stays where it is.
pub fn remove(project: &Path, name: &str) -> Result<String, String> {
    let file = project.join("mote.toml");
    let text = fs::read_to_string(&file).map_err(|error| format!("cannot read {}: {error}", shown(&file)))?;
    let manifest = Manifest::parse(&shown(&file), &text).map_err(|error| error.to_string())?;
    let Some(dep) = manifest.dependencies.iter().find(|dep| dep.name == name) else { return Err(format!("`{name}` is not a dependency")) };
    let mut lines: Vec<&str> = text.lines().collect();
    lines.remove(dep.line as usize - 1);
    fs::write(&file, lines.join("\n") + "\n").map_err(|error| format!("cannot write {}: {error}", shown(&file)))?;
    Ok(match dep.source {
        Source::Zip => format!("removed {name}; {} is still there\n", dep.path),
        Source::Path => format!("removed {name}\n"),
    })
}

#[cfg(test)]
mod tests;
