//! The scripts that build and test a core library, and programs that import the fixture libraries of `tools/fixtures`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use mote::{execute_at, Outcome};
use mote_packages::testing::Scratch;

fn run(line: &str, dir: &Path, home: &Path) -> Outcome {
    let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
    execute_at(&words, dir, home)
}

/// An app that prints what `ping.add` gives.
fn adder(scratch: &Scratch) -> PathBuf {
    scratch.package("app", "app", &[], &[("main.mote", "import { add } from ping\n\nfn main() {\n    println(add(20, 22))\n}\n")])
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Copies a directory, keeping the permissions of its files.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A scratch copy of a fixture set, so the repository gets no built shared libraries.
fn fixtures(scratch: &Scratch, set: &str) -> PathBuf {
    let to = scratch.path(set);
    copy_tree(&root().join("tools/fixtures").join(set), &to);
    to
}

fn script(name: &str, args: &[&str], envs: &[(&str, &Path)]) -> Output {
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(root().join("tools").join(name));
    #[cfg(target_os = "windows")]
    let mut command = {
        let ps1 = name.strip_suffix(".sh").map(|stem| format!("{stem}.ps1")).unwrap_or_else(|| name.to_string());
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(root().join("tools").join(ps1));
        cmd
    };
    command.args(args).current_dir(root());
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

/// The `mote` binary of this build, built once for all the tests of the run.
fn mote() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let mut args = vec!["build", "--quiet", "--offline", "-p", "mote"];
            if !cfg!(debug_assertions) {
                args.push("--release");
            }
            let built = Command::new(cargo).args(&args).current_dir(root()).status().unwrap();
            assert!(built.success(), "cargo build -p mote failed");
            let binary = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join(format!("mote{}", std::env::consts::EXE_SUFFIX));
            assert!(binary.is_file(), "{} is missing", binary.display());
            binary
        })
        .clone()
}

#[test]
fn core_tools_check_passes_when_each_external_library_has_a_build_script() {
    let scratch = Scratch::new();
    let core = fixtures(&scratch, "core");
    let output = script("build-std-libs.sh", &["--check", "--core", core.to_str().unwrap()], &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"), "ok: 3 libraries, 1 with a shared library\n");
    assert!(!core.join("ping/lib").join(mote_packages::host_triple()).exists());
}

#[test]
fn core_tools_check_names_a_library_whose_external_library_has_no_build_script() {
    let scratch = Scratch::new();
    let core = fixtures(&scratch, "core-broken");
    let output = script("build-std-libs.sh", &["--check", "--core", core.to_str().unwrap()], &[]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    #[cfg(not(target_os = "windows"))]
    assert_eq!(stderr, "orphan: declares an external library and has no lib/build.sh or rust/Cargo.toml\n");
    #[cfg(target_os = "windows")]
    assert!(stderr.contains("orphan: declares an external library and has no"), "{stderr}");
}

#[test]
fn core_tools_build_writes_the_shared_library_and_the_archive() {
    let scratch = Scratch::new();
    let core = fixtures(&scratch, "core");
    let output = script("build-std-libs.sh", &["--core", core.to_str().unwrap()], &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let libs = core.join("ping/lib").join(mote_packages::host_triple());
    assert!(libs.join(mote_packages::library_file("ping")).is_file());
    assert!(libs.join(mote_packages::archive_file("ping")).is_file());
    assert!(!core.join("plain/lib").exists());
}

#[test]
fn core_tools_test_core_builds_the_library_then_runs_the_tests() {
    let scratch = Scratch::new();
    let core = fixtures(&scratch, "core");
    let binary = mote();
    for name in ["ping", "plain"] {
        let output = script("test-core.sh", &[name, "--core", core.to_str().unwrap()], &[("MOTE", &binary)]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{name}: {stdout}{}", String::from_utf8_lossy(&output.stderr));
        assert!(stdout.contains("1 passed; 0 failed"), "{name}: {stdout}");
    }
}

#[test]
fn core_tools_test_core_builds_the_external_libraries_it_imports() {
    let scratch = Scratch::new();
    let core = fixtures(&scratch, "core");
    let output = script("test-core.sh", &["pinger", "--core", core.to_str().unwrap()], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("1 passed; 0 failed"), "{stdout}");
    assert!(core.join("ping/lib").join(mote_packages::host_triple()).join(mote_packages::library_file("ping")).is_file());
}

/// `tools/test-core.sh NAME` on the library of the repository, which must pass.
fn library_passes(name: &str) {
    let output = script("test-core.sh", &[name], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 failed"), "{stdout}");
}

#[test]
fn core_path_the_tests_of_the_library_pass() {
    library_passes("path");
}

#[test]
fn core_log_the_tests_of_the_library_pass() {
    library_passes("log");
}

#[test]
fn core_base64_the_tests_of_the_library_pass() {
    library_passes("base64");
}

#[test]
fn core_json_the_tests_of_the_library_pass() {
    library_passes("json");
}

#[test]
fn core_fs_the_tests_of_the_library_pass() {
    library_passes("fs");
}

/// Builds the shared library of the library of the repository, links `source` against it into an executable and gives what it prints.
fn built_program_prints(library: &str, source: &str) -> String {
    let built = script("build-std-libs.sh", &[library], &[]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let app = scratch.package("app", "app", &[], &[("main.mote", source)]);
    let built = run(&format!("build --core {}", root().join("mote_std").display()), &app, &home);
    assert_eq!(built.code, 0, "{built:?}");
    let ran = Command::new(app.join(format!("target/app{}", std::env::consts::EXE_SUFFIX))).output().unwrap();
    assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

#[test]
fn core_fs_a_built_program_links_the_archive_and_reads_a_file() {
    let data = std::env::temp_dir().join(format!("mote-fs-built-{}.txt", std::process::id()));
    let source = format!("import {{ read_string, write_string }} from fs\n\nfn main() -> Result<Null, Error> {{\n    write_string(\"{0}\", \"stored\")?\n    println(read_string(\"{0}\")?)\n    return Ok(null)\n}}\n", mote_packages::shown(&data));
    assert_eq!(built_program_prints("fs", &source), "stored\n");
    fs::remove_file(data).unwrap();
}

#[test]
fn core_env_the_tests_of_the_library_pass() {
    library_passes("env");
}

#[test]
fn core_env_a_built_program_links_the_archive_and_reads_the_environment() {
    #[cfg(not(target_os = "windows"))]
    let probe = "HOME";
    #[cfg(target_os = "windows")]
    let probe = "USERPROFILE";
    let source = format!("import {{ get, set }} from env\n\nfn main() -> Result<Null, Error> {{\n    set(\"MOTE_ENV_BUILT\", \"yes\")?\n    println(get(\"MOTE_ENV_BUILT\")!)\n    println(get(\"{probe}\") != null)\n    return Ok(null)\n}}\n");
    assert_eq!(built_program_prints("env", &source), "yes\ntrue\n");
}

#[test]
fn core_random_the_tests_of_the_library_pass() {
    library_passes("random");
}

#[test]
fn core_random_a_built_program_links_the_archive_and_draws_bytes() {
    let source = "import { Random, system_bytes } from random\n\nfn main() -> Result<Null, Error> {\n    let bytes = system_bytes(12)?\n    println(bytes.len())\n    var random = Random.seeded(0)\n    println(random.next_bits())\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("random", source), "12\n5987356902031041503\n");
}

#[test]
fn core_term_the_tests_of_the_library_pass() {
    library_passes("term");
}

#[test]
fn core_term_a_built_program_links_the_archive_and_reads_a_key() {
    let source = "import { Pty } from term\n\nfn main() -> Result<Null, Error> {\n    let pty = Pty.open()?\n    var terminal = pty.terminal()\n    terminal.raw()?\n    pty.send(\"\\u{1b}[1;5A\")?\n    let event = terminal.read_event(1000)?\n    println(event!)\n    terminal.close()?\n    pty.close()?\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("term", source), "Ctrl+Up\n");
}

#[test]
fn core_tile_the_tests_of_the_library_pass() {
    library_passes("tile");
}

#[test]
fn core_tile_the_builder_tests_pass() {
    let output = script("test-core.sh", &["tile", "--", "builders"], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 failed"), "{stdout}");
}

#[test]
fn core_tile_a_built_program_runs_a_loop_on_a_pseudo_terminal() {
    let source = "import { Pty } from term\nimport { Input, Next, Options, View, run } from tile\n\nfn main() -> Result<Null, Error> {\n    let pty = Pty.open()?\n    pty.resize(3, 10)?\n    pty.send(\"q\")?\n    let seen = run(pty.terminal(), 0, |n: Int| View.text(\"n=${n}\"), |n: Int, input: Input| Next.Quit(n + 1), Options {})?\n    println(seen)\n    pty.close()?\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("term", source), "1\n");
}

#[test]
fn core_net_the_tests_of_the_library_pass() {
    library_passes("net");
}

#[test]
fn core_doc_the_tests_of_the_library_pass() {
    library_passes("doc");
}

#[test]
fn core_doc_a_built_program_links_the_archive_and_writes_html() {
    let source = "import doc\nimport Node from doc\n\nfn main() -> Result<Null, Error> {\n    let node = doc.p([\"a < b\"], class_name=\"lead\")\n    let again = Node.from_bytes(node.bytes())?\n    println(again.to_html())\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("doc", source), "<p class=\"lead\">a &lt; b</p>\n");
}

#[test]
fn core_markdown_the_tests_of_the_library_pass() {
    library_passes("markdown");
}

#[test]
fn core_markdown_a_built_program_links_both_archives_and_renders_text() {
    let source = "import { parse, render, to_markdown } from markdown\n\nfn main() -> Result<Null, Error> {\n    println(render(\"# Hi\\n\\nsome *text*\")?)\n    let root = parse(\"- a\\n- b\")?\n    println(to_markdown(root))\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("markdown", source), "<h1>Hi</h1><p>some <em>text</em></p>\n- a\n- b\n\n");
}

#[test]
fn core_toml_the_tests_of_the_library_pass() {
    library_passes("toml");
}

#[test]
fn core_toml_a_built_program_links_the_archive_and_reads_toml() {
    let source = "import { parse } from toml\n\nfn main() -> Result<Null, Error> {\n    let table = parse(\"name = \\\"mote\\\"\\n[build]\\njobs = 4\\n\")?\n    println(table.get(\"build\")!.get(\"jobs\")!.as_int()!)\n    println(table.to_text()?)\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("toml", source), "4\nname = \"mote\"\n\n[build]\njobs = 4\n\n");
}

#[test]
fn core_yaml_the_tests_of_the_library_pass() {
    library_passes("yaml");
}

#[test]
fn core_yaml_a_built_program_links_the_archive_and_reads_yaml() {
    let source = "import { parse, to_text } from yaml\n\nfn main() -> Result<Null, Error> {\n    let doc = parse(\"name: mote\\nsteps:\\n  - build\\n  - test\\n\")?\n    println(doc.get(\"steps\")!.at(1)!.as_text()!)\n    println(to_text(doc)?)\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("yaml", source), "test\nname: mote\nsteps:\n- build\n- test\n\n");
}

#[test]
fn core_process_the_tests_of_the_library_pass() {
    library_passes("process");
}

#[test]
fn core_process_a_built_program_links_the_archive_and_runs_a_child() {
    let built = script("build-std-libs.sh", &["env"], &[]);
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    #[cfg(not(target_os = "windows"))]
    let source = "import { command } from process\n\nfn main() -> Result<Null, Error> {\n    let out = command(\"echo\").arg(\"from a child\").set_env(\"A\", \"b\").output()?\n    print(out.stdout_string()!)\n    println(out.status.code()!)\n    return Ok(null)\n}\n";
    #[cfg(target_os = "windows")]
    let source = "import { command } from process\n\nfn main() -> Result<Null, Error> {\n    let out = command(\"cmd\").arg(\"/c\").arg(\"echo from a child\").set_env(\"A\", \"b\").output()?\n    print(out.stdout_string()!)\n    println(out.status.code()!)\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("process", source).replace("\r\n", "\n"), "from a child\n0\n");
}

#[test]
fn core_signal_the_tests_of_the_library_pass() {
    library_passes("signal");
}

#[test]
fn core_signal_a_built_program_links_the_archive_and_receives_a_signal() {
    let source = "import { Signal, pid, send, subscribe } from signal\n\nfn main() -> Result<Null, Error> {\n    let sub = subscribe([Signal.User1])?\n    send(pid(), Signal.User1)?\n    println(sub.next()!.number())\n    sub.close()?\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("signal", source), "10\n");
}

#[test]
fn core_regex_the_tests_of_the_library_pass() {
    library_passes("regex");
}

#[test]
fn core_regex_a_built_program_links_the_archive_and_matches() {
    let source = "import { Regex } from regex\n\nfn main() -> Result<Null, Error> {\n    let re = Regex.new(\"(?P<k>[a-z]+)=(?P<v>[0-9]+)\")?\n    let found = re.captures(\"size=42\")!\n    println(found.named(\"v\")!)\n    println(re.replace_all(\"a=1 b=2\", \"$v:$k\"))\n    re.close()?\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("regex", source), "42\n1:a 2:b\n");
}

#[test]
fn core_net_a_built_program_links_the_archive_and_talks_over_a_socket() {
    let source = "import { TcpListener, TcpStream } from net.socket\n\nfn main() -> Result<Null, Error> {\n    let listener = TcpListener.bind(\"127.0.0.1\", 0)?\n    let port = listener.port()?\n    let server = spawn {\n        let stream = listener.accept()!\n        stream.write_string(\"hi\")!\n        stream.close()!\n        return 0\n    }\n    let client = TcpStream.connect(\"localhost\", port)?\n    let bytes = client.read_all()?\n    println(String.from_bytes(bytes)!)\n    server.await()?\n    client.close()?\n    listener.close()?\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("net", source), "hi\n");
}

#[test]
fn core_net_a_built_program_links_the_tls_archive_and_shakes_hands() {
    let tests = std::fs::read_to_string(root().join("mote_std/net/tests/keys.mote")).unwrap();
    let keys = tests.strip_prefix("import join from std.string\n").unwrap();
    let main = "import { join } from std.string\nimport { TcpListener } from net.socket\nimport { TlsAcceptor, TlsStream } from net.tls\n\nfn main() -> Result<Null, Error> {\n    let acceptor = TlsAcceptor.new(certificate(), private_key())?\n    let listener = TcpListener.bind(\"127.0.0.1\", 0)?\n    let port = listener.port()?\n    let server = spawn {\n        let secure = acceptor.accept(listener.accept()!)!\n        let got = secure.read(16)!\n        secure.write(got)!\n        secure.close()!\n        return 0\n    }\n    let client = TlsStream.connect_trusting(\"localhost\", port, authority())?\n    client.write_string(\"secret\")?\n    println(String.from_bytes(client.read_all()?)!)\n    server.await()?\n    client.close()?\n    listener.close()?\n    return Ok(null)\n}\n\n";
    assert_eq!(built_program_prints("net", &format!("{main}{keys}")), "secret\n");
}

#[test]
fn core_date_the_tests_of_the_library_pass() {
    library_passes("date");
}

#[test]
fn core_uuid_the_tests_of_the_library_pass() {
    library_passes("uuid");
}

#[test]
fn core_compress_the_tests_of_the_library_pass() {
    library_passes("compress");
}

#[test]
fn core_compress_a_built_program_links_the_archive_and_round_trips() {
    let source = "import { Format, compress, decompress } from compress\n\nfn main() -> Result<Null, Error> {\n    let packed = compress(Format.Gzip, \"hello hello hello hello\".bytes())?\n    println(packed.len())\n    println(String.from_bytes(decompress(Format.Gzip, packed)?)!)\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("compress", source), "28\nhello hello hello hello\n");
}

#[test]
fn core_pane_the_layout_tests_pass() {
    let output = script("test-core.sh", &["pane", "--", "layout"], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 failed"), "{stdout}");
}

#[test]
fn core_pane_a_built_program_links_the_archive_and_lays_out_a_view() {
    let source = "import doc\nimport { layout } from pane\n\nfn main() -> Result<Null, Error> {\n    let row = doc.el(\"row\", [\n        doc.el(\"box\", id=\"b\", attrs=[(\"width\", \"100\")]),\n        doc.el(\"spacer\")\n    ])\n    let result = layout(row, 300, 50)?\n    let area = result.area(\"b\")!\n    println(\"${area.w}x${area.h}\")\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("pane", source), "100x0\n");
}

#[test]
fn core_pane_the_window_tests_pass() {
    let output = script("test-core.sh", &["pane", "--", "window"], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 failed"), "{stdout}");
}

#[test]
fn core_pane_the_pipeline_tests_pass() {
    let output = script("test-core.sh", &["pane", "--", "pipelines"], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 failed"), "{stdout}");
}

#[test]
fn core_pane_the_builder_tests_pass() {
    let output = script("test-core.sh", &["pane", "--", "builders"], &[("MOTE", &mote())]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.contains("0 failed"), "{stdout}");
}

#[test]
fn core_pane_a_built_program_runs_a_headless_window_through_the_linked_init() {
    let source = "import doc\nimport Node from doc\nimport { Event, Next, Window } from pane\n\nfn view(count: Int) -> Node {\n    return doc.el(\"box\", id=\"b\")\n}\n\nfn step(count: Int, event: Event) -> Next<Int> {\n    return Next.Continue(count + 1)\n}\n\nfn main() -> Result<Null, Error> {\n    var window = Window.headless(200, 100)\n    window.inject(Event.Click(null, 5, 5, 0))\n    window.inject(Event.Key(65))\n    let count = window.run(0, view, step)!\n    println(count)\n    println(window.frames.len())\n    window.close()\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("pane", source), "2\n3\n");
}

#[test]
fn core_args_the_tests_of_the_library_pass() {
    library_passes("args");
}

#[test]
fn core_args_a_built_program_reads_its_arguments_and_writes_its_usage() {
    let source = "import { Args, Flag, flag, from_env, usage_text } from args\n\n/// Counts words.\nclass Opts: (Args) {\n    /// How many.\n    count: Int\n    loud: Bool\n\n    fn flags() -> List<Flag> { return [flag(\"count\").short(\"n\").default_to(\"3\")] }\n}\n\nfn main() -> Result<Null, Error> {\n    let opts = from_env<Opts>()?\n    println(opts.count)\n    println(opts.loud)\n    print(usage_text<Opts>(\"count\")?)\n    return Ok(null)\n}\n";
    let usage = "Usage: count [OPTIONS]\n\nCounts words.\n\nOptions:\n  -n, --count <INT>  How many. [default: 3]\n      --loud\n  -h, --help         Show this help\n";
    assert_eq!(built_program_prints("args", source), format!("3\nfalse\n{usage}"));
}

#[test]
fn core_sqlite_the_tests_of_the_library_pass() {
    library_passes("sqlite");
}

#[test]
fn core_sqlite_a_built_program_links_the_archive_and_queries() {
    let source = "import { Database, Value } from sqlite\n\nfn main() -> Result<Null, Error> {\n    let db = Database.memory()?\n    db.execute_batch(\"CREATE TABLE t (k TEXT, n INTEGER)\")?\n    db.execute(\"INSERT INTO t VALUES (?, ?)\", [Value.Text(\"one\"), Value.Integer(1)])?\n    let found = db.query_row(\"SELECT k, n FROM t\")?\n    let row = found!\n    println(row.get(0).as_text()!)\n    println(row.get(1).as_int()!)\n    db.close()?\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("sqlite", source), "one\n1\n");
}

#[test]
fn core_archive_the_tests_of_the_library_pass() {
    library_passes("archive");
}

#[test]
fn core_archive_a_built_program_links_the_archives_and_packs_entries() {
    for library in ["fs", "compress"] {
        let built = script("build-std-libs.sh", &[library], &[]);
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    }
    let source = "import { Entry, Format, read, write } from archive\n\nfn main() -> Result<Null, Error> {\n    let packed = write(Format.TarGz, [Entry.file(\"a.txt\", \"hello\".bytes()), Entry.directory(\"d\")])?\n    let entries = read(Format.TarGz, packed)?\n    println(entries.len())\n    println(entries.get(0).text()!)\n    let zipped = read(Format.Zip, write(Format.Zip, entries)?)?\n    println(zipped.get(1).path)\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("archive", source), "2\nhello\nd\n");
}

#[test]
fn core_crypto_the_tests_of_the_library_pass() {
    library_passes("crypto");
}

#[test]
fn core_crypto_a_built_program_links_the_archive_and_hashes() {
    let source = "import { Algorithm, hash_text, hmac } from crypto\n\nfn main() -> Result<Null, Error> {\n    println(hash_text(Algorithm.Sha256, \"abc\"))\n    println(hmac(Algorithm.Sha256, \"Jefe\".bytes(), \"what do ya want for nothing?\".bytes()).to_hex())\n    return Ok(null)\n}\n";
    assert_eq!(built_program_prints("crypto", source), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843\n");
}

#[test]
fn core_path_a_program_imports_it_with_no_manifest_entry() {
    let scratch = Scratch::new();
    let source = "import { extension, join, normalize, relative_to } from path\n\nfn main() {\n    let full = normalize(join(\"/srv/app\", \"../data/./list.txt\"))\n    println(full)\n    println(extension(full)!)\n    println(relative_to(full, \"/srv\")!)\n}\n";
    let dir = scratch.package("app", "app", &[], &[("main.mote", source)]);
    assert_eq!(run("run", &dir, &scratch.path("home")), Outcome::ok("/srv/data/list.txt\ntxt\ndata/list.txt\n"));
}

#[test]
fn core_tools_test_core_of_an_unknown_library_is_an_error() {
    let scratch = Scratch::new();
    let core = fixtures(&scratch, "core");
    let output = script("test-core.sh", &["missing", "--core", core.to_str().unwrap()], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no library missing in "));
}

/// Copies the fixture libraries into the scratch directory and builds their external libraries.
fn built_core(scratch: &Scratch) -> PathBuf {
    let core = fixtures(scratch, "core");
    let output = script("build-std-libs.sh", &["--core", core.to_str().unwrap()], &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    core
}

#[test]
fn core_tools_an_app_imports_a_core_library_by_name_and_runs() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let core = built_core(&scratch);
    let app = adder(&scratch);
    assert_eq!(run(&format!("run --core {}", core.display()), &app, &home), Outcome::ok("42\n"));
    assert_eq!(run(&format!("check --core {}", core.display()), &app, &home).code, 0);
}

#[test]
fn core_tools_a_core_name_without_the_library_is_an_unknown_package() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let core = fixtures(&scratch, "core");
    let app = scratch.package("app", "app", &[], &[("main.mote", "import { add } from absent\n\nfn main() {}\n")]);
    let outcome = run(&format!("check --core {}", core.display()), &app, &home);
    assert_eq!(outcome.code, 1);
    assert!(outcome.stderr.contains("no package `absent`"), "{}", outcome.stderr);
}

#[test]
fn core_tools_an_app_named_as_a_core_library_is_reserved() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let core = fixtures(&scratch, "core");
    let app = scratch.package("app", "ping", &[], &[("main.mote", "fn main() {}\n")]);
    let outcome = run(&format!("check --core {}", core.display()), &app, &home);
    assert_eq!(outcome.code, 1);
    assert!(outcome.stderr.ends_with("`package.name`: `ping` is reserved\n"), "{}", outcome.stderr);
}

#[test]
fn core_tools_a_built_program_carries_the_shared_library_it_opens_beside_it() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let core = built_core(&scratch);
    fs::remove_file(core.join("ping/lib").join(mote_packages::host_triple()).join(mote_packages::archive_file("ping"))).unwrap();
    let app = adder(&scratch);
    assert_eq!(run(&format!("build --core {}", core.display()), &app, &home).code, 0);
    assert!(app.join("target/lib").join(mote_packages::library_file("ping")).is_file());
    fs::remove_dir_all(&core).unwrap();
    let ran = Command::new(app.join(format!("target/app{}", std::env::consts::EXE_SUFFIX))).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "42\n", "{}", String::from_utf8_lossy(&ran.stderr));
}

#[test]
fn core_tools_a_built_program_with_a_linked_archive_needs_no_file_beside_it() {
    let scratch = Scratch::new();
    let home = scratch.path("home");
    let core = built_core(&scratch);
    let app = adder(&scratch);
    assert_eq!(run(&format!("build --core {}", core.display()), &app, &home).code, 0);
    assert!(!app.join("target/lib").exists());
    fs::remove_dir_all(&core).unwrap();
    let ran = Command::new(app.join(format!("target/app{}", std::env::consts::EXE_SUFFIX))).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "42\n", "{}", String::from_utf8_lossy(&ran.stderr));
}

/// The `.mote` files below a directory, with their paths relative to it.
fn mote_files(dir: &Path, below: &str, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = format!("{below}{}", entry.file_name().to_string_lossy());
        if entry.file_type().unwrap().is_dir() {
            mote_files(&entry.path(), &format!("{name}/"), found);
        } else if name.ends_with(".mote") {
            found.push(name);
        }
    }
}

/// Every non-`std` library of `mote_std/` with the files of its `src/`.
fn core_sources() -> Vec<(String, Vec<String>)> {
    let mut libraries = Vec::new();
    for entry in fs::read_dir(root().join("mote_std")).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.join("mote.toml").is_file() || dir.file_name().is_some_and(|name| name == "std") {
            continue;
        }
        let mut files = Vec::new();
        mote_files(&dir.join("src"), "", &mut files);
        files.sort();
        libraries.push((dir.file_name().unwrap().to_string_lossy().to_string(), files));
    }
    libraries.sort();
    libraries
}

#[test]
fn core_layout_lib_mote_holds_only_reexports() {
    let mut problems = Vec::new();
    for (library, _) in core_sources() {
        let text = fs::read_to_string(root().join("mote_std").join(&library).join("src/lib.mote")).unwrap();
        for (at, line) in text.lines().enumerate() {
            let line = line.trim();
            if !(line.is_empty() || line.starts_with("//") || line.starts_with("pub import ")) {
                problems.push(format!("mote_std/{library}/src/lib.mote:{}: {line}", at + 1));
            }
        }
    }
    assert!(problems.is_empty(), "lib.mote may hold comments and `pub import` lines only:\n{}", problems.join("\n"));
}

#[test]
fn core_layout_externs_follow_lib_convention() {
    let mut problems = Vec::new();
    for (library, files) in core_sources() {
        for file in files {
            let text = fs::read_to_string(root().join("mote_std").join(&library).join("src").join(&file)).unwrap();
            let name = file.rsplit('/').next().unwrap();
            if name.contains("native") {
                problems.push(format!("mote_std/{library}/src/{file}: legacy `_native` filename"));
            }
            let lib_file = name == "_lib.mote";
            let mut in_extern = false;
            for (at, line) in text.lines().enumerate() {
                let trimmed = line.trim();
                let opens_extern = trimmed.starts_with("extern \"") && trimmed.contains('{');
                if line.starts_with("native fn ") || line.starts_with("pub native fn ") || line.starts_with("@link(") {
                    problems.push(format!("mote_std/{library}/src/{file}:{}: legacy native fn syntax", at + 1));
                }
                if opens_extern {
                    if !trimmed.ends_with('}') {
                        in_extern = true;
                    }
                    continue;
                }
                if in_extern {
                    if trimmed == "}" {
                        in_extern = false;
                    }
                    continue;
                }
                let other = !(trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("import ") || trimmed.starts_with("pub import ") || trimmed.starts_with('@') || trimmed.starts_with("let ") || trimmed.starts_with("pub let ") || trimmed.starts_with("const ") || trimmed.starts_with("pub const "));
                if lib_file && other {
                    problems.push(format!("mote_std/{library}/src/{file}:{}: a `_lib.mote` file holds extern declarations and constants only: {line}", at + 1));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn core_layout_no_file_is_longer_than_450_lines() {
    let mut problems = Vec::new();
    for (library, files) in core_sources() {
        for file in files {
            let lines = fs::read_to_string(root().join("mote_std").join(&library).join("src").join(&file)).unwrap().lines().count();
            if lines > 450 {
                problems.push(format!("mote_std/{library}/src/{file}: {lines} lines"));
            }
        }
    }
    assert!(problems.is_empty(), "split these by topic:\n{}", problems.join("\n"));
}
