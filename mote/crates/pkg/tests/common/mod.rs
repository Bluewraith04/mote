//! A small fake registry on localhost, shared by the registry tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pkg::registry::archive_checksum;
use pkg::MpkArchive;

/// The token the fake accepts on a publish.
pub const TOKEN: &str = "secret";

/// Points the mote home at a directory of this test run, so nothing lands in the user's own.
pub fn isolate_home() {
    static HOME: std::sync::Once = std::sync::Once::new();
    HOME.call_once(|| std::env::set_var("MOTE_HOME", std::env::temp_dir().join(format!("mote_home_registry_{}", std::process::id()))));
}

pub struct Published {
    pub version: String,
    pub bytes: Vec<u8>,
    pub yanked: bool,
    pub deps: Vec<(String, String)>,
}

#[derive(Default)]
pub struct State {
    pub packages: BTreeMap<String, Vec<Published>>,
    /// Requests, in order, as `METHOD /path`.
    pub hits: Vec<String>,
    /// Serve archives with a flipped byte.
    pub tamper: bool,
    /// Answer a publish with this checksum instead of the real one.
    pub wrong_checksum: bool,
}

/// A registry on a free local port, serving what was published to it.
pub struct Fake {
    pub url: String,
    pub state: Arc<Mutex<State>>,
    stop: Arc<Mutex<bool>>,
    port: u16,
}

impl Fake {
    pub fn start() -> Fake {
        isolate_home();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(Mutex::new(false));
        let (s, st) = (state.clone(), stop.clone());
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                if *st.lock().unwrap() {
                    break;
                }
                if let Ok(conn) = conn {
                    let s = s.clone();
                    std::thread::spawn(move || handle(conn, &s));
                }
            }
        });
        Fake { url: format!("http://127.0.0.1:{port}"), state, stop, port }
    }

    /// Publishes `name` at `version` with dependencies; answers the archive's checksum.
    pub fn publish(&self, name: &str, version: &str, deps: &[(&str, &str)], lib: &str) -> String {
        self.publish_files(name, version, deps, &[("src/lib.mote", lib)])
    }

    pub fn publish_files(&self, name: &str, version: &str, deps: &[(&str, &str)], files: &[(&str, &str)]) -> String {
        let lines: String = deps.iter().map(|(n, r)| format!("{n} = \"{r}\"\n")).collect();
        let manifest = format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"src/lib.mote\"\n\n[dependencies]\n{lines}");
        let mut entries = vec![("mote.toml".to_string(), manifest.into_bytes()), (format!("{name}.mbc"), b"bytecode".to_vec()), ("interface.txt".to_string(), b"// api".to_vec())];
        entries.extend(files.iter().map(|(p, c)| (p.to_string(), c.as_bytes().to_vec())));
        self.publish_archive(name, version, deps, MpkArchive::encode(&entries).unwrap())
    }

    pub fn publish_archive(&self, name: &str, version: &str, deps: &[(&str, &str)], bytes: Vec<u8>) -> String {
        let sum = archive_checksum(&bytes);
        let deps = deps.iter().map(|(n, r)| (n.to_string(), r.to_string())).collect();
        self.state.lock().unwrap().packages.entry(name.to_string()).or_default().push(Published { version: version.to_string(), bytes, yanked: false, deps });
        sum
    }

    pub fn yank(&self, name: &str, version: &str) {
        let mut state = self.state.lock().unwrap();
        state.packages.get_mut(name).unwrap().iter_mut().find(|p| p.version == version).unwrap().yanked = true;
    }

    pub fn hits(&self) -> Vec<String> {
        self.state.lock().unwrap().hits.clone()
    }

    /// The archive stored for `name` at `version`.
    pub fn archive(&self, name: &str, version: &str) -> Option<Vec<u8>> {
        let state = self.state.lock().unwrap();
        state.packages.get(name)?.iter().find(|p| p.version == version).map(|p| p.bytes.clone())
    }

    /// Stops answering: later connections are refused.
    pub fn stop(&self) {
        *self.stop.lock().unwrap() = true;
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    /// A project depending on `deps` (lines of `[dependencies]`), with this registry named `main`.
    pub fn app(&self, name: &str, deps: &str) -> PathBuf {
        let app = std::env::temp_dir().join(format!("mote_registry_{name}_{}", std::process::id()));
        fs::remove_dir_all(&app).ok();
        fs::create_dir_all(app.join("src")).unwrap();
        fs::write(app.join("mote.toml"), format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n{deps}\n[registries]\nmain = \"{}\"\n", self.url)).unwrap();
        fs::write(app.join("src/main.mote"), "fn main() {}\n").unwrap();
        app
    }
}

fn handle(mut conn: TcpStream, state: &Mutex<State>) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if conn.read(&mut byte).unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.lines();
    let request = lines.next().unwrap_or("").to_string();
    let mut parts = request.split(' ');
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
    let length: usize = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; length];
    conn.read_exact(&mut body).ok();
    let (status, content) = {
        let mut state = state.lock().unwrap();
        state.hits.push(format!("{method} {path}"));
        route(&mut state, &method, &path, header("authorization").as_deref(), &body)
    };
    let reply = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", content.len());
    let _ = conn.write_all(reply.as_bytes());
    let _ = conn.write_all(&content);
}

fn route(state: &mut State, method: &str, path: &str, auth: Option<&str>, body: &[u8]) -> (&'static str, Vec<u8>) {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["index", a, b, name]) if name.starts_with(a) && name[1..].starts_with(b) => match state.packages.get(*name) {
            Some(versions) => {
                let lines: String = versions
                    .iter()
                    .map(|p| {
                        let deps: BTreeMap<&String, &String> = p.deps.iter().map(|(n, r)| (n, r)).collect();
                        format!("{}\n", serde_json::json!({ "version": p.version, "checksum": archive_checksum(&p.bytes), "yanked": p.yanked, "dependencies": deps }))
                    })
                    .collect();
                ("200 OK", lines.into_bytes())
            }
            None => ("404 Not Found", b"no such package".to_vec()),
        },
        ("GET", ["dl", name, file]) => {
            let found = state.packages.get(*name).and_then(|vs| vs.iter().find(|p| format!("{}.mpk", p.version) == *file));
            match found {
                Some(p) => {
                    let mut bytes = p.bytes.clone();
                    if state.tamper {
                        let at = bytes.len() / 2;
                        bytes[at] ^= 0xff;
                    }
                    ("200 OK", bytes)
                }
                None => ("404 Not Found", b"no such archive".to_vec()),
            }
        }
        ("PUT", ["publish"]) => upload(state, auth, body),
        _ => ("404 Not Found", b"not found".to_vec()),
    }
}

fn upload(state: &mut State, auth: Option<&str>, body: &[u8]) -> (&'static str, Vec<u8>) {
    if auth != Some(&format!("Bearer {TOKEN}")) {
        return ("401 Unauthorized", b"the token is not accepted".to_vec());
    }
    let Ok(entries) = MpkArchive::decode(body) else { return ("400 Bad Request", b"not an archive".to_vec()) };
    let Some((_, toml)) = entries.iter().find(|(p, _)| p == "mote.toml") else { return ("400 Bad Request", b"no mote.toml".to_vec()) };
    let Ok(manifest) = pkg::PackageManifest::from_toml_str(&String::from_utf8_lossy(toml)) else { return ("400 Bad Request", b"bad mote.toml".to_vec()) };
    let (name, version) = (manifest.package.name.clone(), manifest.package.version.to_string());
    let versions = state.packages.entry(name.clone()).or_default();
    if versions.iter().any(|p| p.version == version) {
        return ("409 Conflict", format!("{name} {version} is already published").into_bytes());
    }
    let deps = manifest.dependencies.iter().map(|(k, v)| (k.clone(), v.version_req_str().to_string())).collect();
    versions.push(Published { version: version.clone(), bytes: body.to_vec(), yanked: false, deps });
    let checksum = if state.wrong_checksum { "sha256:00".to_string() } else { archive_checksum(body) };
    ("201 Created", serde_json::json!({ "name": name, "version": version, "checksum": checksum }).to_string().into_bytes())
}
