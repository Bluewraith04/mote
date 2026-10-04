use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use compiler::ast::{ModulePath as AstModulePath, Program};
use compiler::lexer::Lexer;
use compiler::parser::Parser;
use crate::path::CanonicalModuleId;

/// Finds, reads and parses the modules an entry file imports.
pub struct ModuleResolver {
    search_paths: Vec<PathBuf>,
    ast_cache: HashMap<CanonicalModuleId, Program>,
    resolved_paths: HashMap<CanonicalModuleId, PathBuf>,
    source_cache: HashMap<CanonicalModuleId, String>,
    cache_dir: Option<PathBuf>,
}

fn parse_source(content: &str, name: &str) -> Result<(u32, Program), String> {
    let mut lexer = Lexer::new(content);
    let source_id = lexer.source_id();
    compiler::span::name_source(source_id, name);
    let tokens = lexer.tokenize().map_err(|(err, span)| format!("Lexer error at line {}:{}: {}", span.line, span.col, err))?;
    let program = Parser::new(tokens).parse().map_err(|(err, span)| format!("Parser error at line {}:{}: {}", span.line, span.col, err))?;
    Ok((source_id, program))
}

fn cache_key(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(env!("MOTE_FRONTEND_FINGERPRINT"));
    hasher.update([0]);
    hasher.update(content);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_cached(dir: &Path, content: &str, name: &str) -> Result<Program, String> {
    let key = cache_key(content);
    let file = dir.join("modules").join(&key[..2]).join(format!("{key}.mote-ir"));
    if let Some(program) = fs::read(&file).ok().and_then(|bytes| load_entry(&bytes, content, name)) {
        return Ok(program);
    }
    let (source_id, program) = parse_source(content, name)?;
    store_entry(&file, source_id, &program);
    Ok(program)
}

fn load_entry(bytes: &[u8], content: &str, name: &str) -> Option<Program> {
    let old = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
    let id = compiler::span::next_source_id();
    if !compiler::span::is_release() {
        compiler::span::register_source_text(id, content);
    }
    compiler::span::name_source(id, name);
    compiler::span::with_source_remap(old, id, || bincode::deserialize(&bytes[4..]).ok())
}

fn store_entry(file: &Path, source_id: u32, program: &Program) {
    let Ok(body) = bincode::serialize(program) else { return };
    let Some(parent) = file.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    let mut bytes = source_id.to_le_bytes().to_vec();
    bytes.extend(body);
    if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

impl ModuleResolver {
    pub fn new(root_dir: PathBuf) -> Self {
        let mut search_paths = vec![root_dir.clone()];
        let pkg_dir = root_dir.join(".mote_packages");
        if pkg_dir.exists() {
            search_paths.push(pkg_dir);
        }
        Self {
            search_paths,
            ast_cache: HashMap::new(),
            resolved_paths: HashMap::new(),
            source_cache: HashMap::new(),
            cache_dir: None,
        }
    }

    /// Resolve an AST module path relative to the importing file's directory.
    pub fn resolve_path(&mut self, ast_path: &AstModulePath, from_dir: &Path) -> Result<(CanonicalModuleId, PathBuf), String> {
        if !ast_path.is_relative && ast_path.segments.first().map(String::as_str) == Some("std") {
            match crate::embedded::resolve(&ast_path.segments) {
                Some(id) => {
                    let path = PathBuf::from(id.as_str());
                    self.resolved_paths.insert(id.clone(), path.clone());
                    return Ok((id, path));
                }
                None => {
                    let name = ast_path.to_dotted_string();
                    let inside = crate::embedded::modules_under(&ast_path.segments);
                    let moved = match ast_path.segments.as_slice() {
                        [_, one] => crate::embedded::moved_to(one),
                        _ => None,
                    };
                    if let Some(package) = crate::embedded::packaged_as(&ast_path.segments) {
                        return Err(format!("unknown standard-library module '{name}': it is now the package `{package}`"));
                    }
                    return Err(match (ast_path.segments.len(), inside.first(), moved) {
                        (1, _, _) => format!("unknown standard-library module '{name}': name one, as in `import std.sys.env`"),
                        (_, _, Some(now)) => format!("unknown standard-library module '{name}': did you mean `{now}`?"),
                        (_, Some(first), _) => format!("unknown standard-library module '{name}': did you mean `{first}`?"),
                        _ => format!("unknown standard-library module '{name}'"),
                    });
                }
            }
        }

        if ast_path.is_relative {
            let mut rel_dir = from_dir.to_path_buf();
            if ast_path.relative_depth > 1 {
                for _ in 1..ast_path.relative_depth {
                    if let Some(parent) = rel_dir.parent() {
                        rel_dir = parent.to_path_buf();
                    }
                }
            }
            for (i, seg) in ast_path.segments.iter().enumerate() {
                if i == ast_path.segments.len() - 1 {
                    if let Some(target) = self.try_resolve_file_or_dir(&rel_dir.join(seg)) {
                        let id = CanonicalModuleId::new(self.canonicalize_id(&target));
                        self.resolved_paths.insert(id.clone(), target.clone());
                        return Ok((id, target));
                    }
                } else {
                    rel_dir = rel_dir.join(seg);
                }
            }
            return Err(format!("Could not resolve relative module '{}' from '{:?}'", ast_path.to_dotted_string(), from_dir));
        }

        for base in &self.search_paths.clone() {
            if let [first, rest @ ..] = ast_path.segments.as_slice() {
                let src = base.join(first).join("src");
                if !rest.is_empty() && src.is_dir() {
                    if let Some(target) = self.resolve_under(&src, rest) {
                        let id = CanonicalModuleId::new(self.canonicalize_id(&target));
                        self.resolved_paths.insert(id.clone(), target.clone());
                        return Ok((id, target));
                    }
                }
            }
            let mut cur = base.clone();
            for (i, seg) in ast_path.segments.iter().enumerate() {
                if i == ast_path.segments.len() - 1 {
                    if let Some(target) = self.try_resolve_file_or_dir(&cur.join(seg)) {
                        let id = CanonicalModuleId::new(self.canonicalize_id(&target));
                        self.resolved_paths.insert(id.clone(), target.clone());
                        return Ok((id, target));
                    }
                    let pkg_target = cur.join(seg).join("src").join("lib.mote");
                    if pkg_target.exists() {
                        let id = CanonicalModuleId::new(self.canonicalize_id(&pkg_target));
                        self.resolved_paths.insert(id.clone(), pkg_target.clone());
                        return Ok((id, pkg_target));
                    }
                } else {
                    cur = cur.join(seg);
                }
            }
        }

        Err(format!("Module '{}' not found in search paths", ast_path.to_dotted_string()))
    }

    /// The module `segments` names under `dir`: a package's `src/` for `import package.module`.
    fn resolve_under(&self, dir: &Path, segments: &[String]) -> Option<PathBuf> {
        let (last, parents) = segments.split_last()?;
        let mut cur = dir.to_path_buf();
        for seg in parents {
            cur = cur.join(seg);
        }
        self.try_resolve_file_or_dir(&cur.join(last))
    }

    fn try_resolve_file_or_dir(&self, base_path: &Path) -> Option<PathBuf> {
        let as_mote_file = base_path.with_extension("mote");
        if as_mote_file.is_file() {
            return Some(as_mote_file);
        }
        let as_mod_file = base_path.join("mod.mote");
        if as_mod_file.is_file() {
            return Some(as_mod_file);
        }
        let as_lib_file = base_path.join("lib.mote");
        if as_lib_file.is_file() {
            return Some(as_lib_file);
        }
        let as_src_lib_file = base_path.join("src").join("lib.mote");
        if as_src_lib_file.is_file() {
            return Some(as_src_lib_file);
        }
        None
    }

    fn canonicalize_id(&self, path: &Path) -> String {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_string()
    }

    /// Parses and caches the program for a resolved module, a source file or an embedded `std.*` module. Every non-`std` module gets an injected `import std.prelude`.
    pub(crate) fn parse_module(&mut self, id: &CanonicalModuleId, path: &Path) -> Result<Program, String> {
        if let Some(program) = self.ast_cache.get(id) {
            return Ok(program.clone());
        }

        let is_embedded = crate::embedded::is_embedded_id(id.as_str());
        let content = if is_embedded {
            crate::embedded::source_for_id(id.as_str())
                .ok_or_else(|| format!("no embedded stdlib module for id '{}'", id.as_str()))?
                .to_string()
        } else {
            fs::read_to_string(path).map_err(|e| format!("Failed to read '{:?}': {}", path, e))?
        };

        let name = id.display_path();
        let mut program = match &self.cache_dir {
            Some(dir) => parse_cached(dir, &content, &name)?,
            None => parse_source(&content, &name)?.1,
        };

        if !is_embedded {
            crate::embedded::inject_prelude_import(&mut program);
        }

        self.source_cache.insert(id.clone(), content);
        self.ast_cache.insert(id.clone(), program.clone());
        Ok(program)
    }

    /// Parses `src` (named `label` in diagnostics) and appends its items to module `id`.
    pub(crate) fn append_source(&mut self, id: &CanonicalModuleId, path: &Path, src: &str, label: &str) -> Result<(), String> {
        self.parse_module(id, path)?;
        let mut lexer = Lexer::new(src);
        compiler::span::name_source(lexer.source_id(), label);
        let tokens = lexer.tokenize().map_err(|(err, span)| format!("Lexer error at line {}:{}: {}", span.line, span.col, err))?;
        let extra = Parser::new(tokens).parse().map_err(|(err, span)| format!("Parser error at line {}:{}: {}", span.line, span.col, err))?;
        self.ast_cache.get_mut(id).expect("parsed above").items.extend(extra.items);
        Ok(())
    }

    /// Reuses parsed modules from `dir/modules`.
    pub(crate) fn set_cache_dir(&mut self, dir: PathBuf) {
        self.cache_dir = Some(dir);
    }

    /// A module's raw source text, available once `parse_module` has run for it.
    pub(crate) fn get_source(&self, id: &CanonicalModuleId) -> Option<&str> {
        self.source_cache.get(id).map(String::as_str)
    }
}
