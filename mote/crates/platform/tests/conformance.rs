use contracts::{Platform, PlatformRequest, PlatformResponse, StdStream};
use platform::{conformance, FakePlatform, SystemPlatform};

fn args() -> Vec<String> {
    vec!["prog".to_string(), "x".to_string()]
}

#[test]
fn test_system_platform_conforms() {
    let scratch = std::env::temp_dir().join(format!("mote_platform_conformance_{}", std::process::id()));
    conformance::check(&SystemPlatform::new(args()), &args(), scratch.to_str().unwrap());
    let _ = std::fs::remove_file(&scratch);
    let _ = std::fs::remove_file(format!("{}.handle", scratch.display()));
    let _ = std::fs::remove_dir_all(format!("{}.dir", scratch.display()));
}

#[test]
fn test_fake_platform_conforms() {
    conformance::check(&FakePlatform::new(7).with_args(args()).with_env("K", "V"), &args(), "scratch");
}

#[test]
fn test_fake_stdin_lines_then_end_of_input() {
    let p = FakePlatform::new(1).with_stdin_line("one");
    assert_eq!(p.execute(PlatformRequest::ReadLine).unwrap(), PlatformResponse::Line(Some("one".to_string())));
    assert_eq!(p.execute(PlatformRequest::ReadLine).unwrap(), PlatformResponse::Line(None));
}

#[test]
fn test_fake_never_blocks_but_system_blocks_on_io() {
    let read = PlatformRequest::ReadLine;
    assert!(!FakePlatform::new(1).blocking(&read));
    assert!(SystemPlatform::new(args()).blocking(&read));
}

#[test]
fn test_fake_clock_moves_only_when_advanced() {
    let p = FakePlatform::new(1);
    let read = |p: &FakePlatform| p.execute(PlatformRequest::MonotonicNanos).unwrap();
    assert_eq!(read(&p), PlatformResponse::Int(0));
    assert_eq!(read(&p), PlatformResponse::Int(0));
    p.advance(2_500_000);
    assert_eq!(read(&p), PlatformResponse::Int(2_500_000));
}

#[test]
fn test_fake_entropy_is_reproducible_per_seed() {
    let draw = |seed| {
        let p = FakePlatform::new(seed);
        (0..3).map(|_| p.execute(PlatformRequest::Entropy).unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(draw(5), draw(5));
    assert_ne!(draw(5), draw(6));
}

#[test]
fn test_fake_captures_output_per_stream() {
    let p = FakePlatform::new(1);
    p.execute(PlatformRequest::Write { stream: StdStream::Stdout, bytes: b"a\n".to_vec() }).unwrap();
    p.execute(PlatformRequest::Write { stream: StdStream::Stderr, bytes: b"e".to_vec() }).unwrap();
    assert_eq!((p.stdout().as_str(), p.stderr().as_str()), ("a\n", "e"));
}

#[test]
fn test_fake_env_lookup() {
    let p = FakePlatform::new(1).with_env("K", "V");
    assert_eq!(
        p.execute(PlatformRequest::EnvVar { name: "K".to_string() }).unwrap(),
        PlatformResponse::OptionString(Some("V".to_string()))
    );
}

#[test]
fn test_system_timer_conforms() {
    conformance::check_timer(&SystemPlatform::new(args()), &|| std::thread::sleep(std::time::Duration::from_millis(3)));
}

#[test]
fn test_fake_timer_conforms() {
    let p = FakePlatform::new(1);
    conformance::check_timer(&p, &|| p.advance(1_000_000));
}

#[test]
fn test_fake_timer_ticks_follow_the_virtual_clock_only() {
    use contracts::{event_queue, EventPayload, Overflow, SourceRequest};
    let p = FakePlatform::new(1);
    let (sink, queue) = event_queue(8, Overflow::Pause);
    let _handle = p.open_source(SourceRequest::Timer { period_nanos: 10 }, sink).unwrap();
    assert!(queue.is_empty());
    p.advance(25);
    assert_eq!((queue.try_pop(), queue.try_pop(), queue.try_pop()), (Some(EventPayload::Int(1)), Some(EventPayload::Int(2)), None));
    p.execute(PlatformRequest::Sleep { nanos: 10 }).unwrap();
    assert_eq!(queue.try_pop(), Some(EventPayload::Int(3)));
}

#[test]
fn test_fake_timer_resumes_after_a_full_queue_drains() {
    use contracts::{event_queue, EventPayload, Overflow, SourceRequest};
    let p = FakePlatform::new(1);
    let (sink, queue) = event_queue(1, Overflow::Pause);
    let _handle = p.open_source(SourceRequest::Timer { period_nanos: 10 }, sink).unwrap();
    p.advance(30);
    assert_eq!(queue.try_pop(), Some(EventPayload::Int(1)));
    assert_eq!(queue.try_pop(), None);
    p.advance(0);
    assert_eq!(queue.try_pop(), Some(EventPayload::Int(2)));
}

#[test]
fn test_fake_emit_end_and_close_are_observable() {
    use contracts::{event_queue, EventPayload, Overflow, PushResult, SourceRequest};
    let p = FakePlatform::new(1);
    let (sink, queue) = event_queue(2, Overflow::Pause);
    let handle = p.open_source(SourceRequest::Timer { period_nanos: 1_000 }, sink).unwrap();
    assert_eq!(p.emit(0, EventPayload::Text("hi".into())), Some(PushResult::Queued));
    assert_eq!(p.emit(9, EventPayload::Unit), None);
    assert_eq!(queue.try_pop(), Some(EventPayload::Text("hi".into())));
    assert!(!p.source_closed(0));
    handle.close();
    assert!(p.source_closed(0));
    p.end_source(0);
    assert!(queue.is_finished());
}

#[cfg(unix)]
#[test]
fn test_system_runs_a_child_and_captures_its_output() {
    let run = |program: &str, argv: &[&str], env: &[&str], stdin: &[u8], cwd: &str| {
        let request = PlatformRequest::RunProcess {
            program: program.to_string(),
            args: argv.iter().map(|s| s.to_string()).collect(),
            env: env.iter().map(|s| s.to_string()).collect(),
            stdin: stdin.to_vec(),
            cwd: cwd.to_string(),
        };
        SystemPlatform::new(args()).execute(request)
    };
    let PlatformResponse::Process(out) = run("sh", &["-c", "echo out; echo err >&2; exit 3"], &[], b"", "").unwrap() else {
        panic!("expected Process");
    };
    assert_eq!((out.status, out.stdout.as_slice(), out.stderr.as_slice()), (3, &b"out\n"[..], &b"err\n"[..]));

    let PlatformResponse::Process(out) = run("cat", &[], &[], b"piped in", "").unwrap() else { panic!() };
    assert_eq!(out.stdout, b"piped in");

    let PlatformResponse::Process(out) = run("sh", &["-c", "printf %s \"$MOTE_X:$PWD\""], &["MOTE_X=1"], b"", "/").unwrap() else {
        panic!()
    };
    assert_eq!(out.stdout, b"1:/");

    let PlatformResponse::Process(out) = run("sh", &["-c", "kill -9 $$"], &[], b"", "").unwrap() else { panic!() };
    assert_eq!(out.status, -1);

    assert_eq!(run("mote-no-such-program-x", &[], &[], b"", "").unwrap_err().kind, contracts::PlatformErrorKind::NotFound);
    assert!(SystemPlatform::new(args()).blocking(&PlatformRequest::RunProcess {
        program: String::new(),
        args: vec![],
        env: vec![],
        stdin: vec![],
        cwd: String::new()
    }));
}

#[test]
fn test_fake_answers_scripted_processes_and_logs_requests() {
    let p = FakePlatform::new(1).with_process("git", 0, b"ok\n", b"");
    let request = |program: &str| PlatformRequest::RunProcess {
        program: program.to_string(),
        args: vec!["status".to_string()],
        env: vec![],
        stdin: vec![],
        cwd: String::new(),
    };
    let PlatformResponse::Process(out) = p.execute(request("git")).unwrap() else { panic!() };
    assert_eq!((out.status, out.stdout), (0, b"ok\n".to_vec()));
    assert_eq!(p.execute(request("hg")).unwrap_err().kind, contracts::PlatformErrorKind::NotFound);
    assert_eq!(p.process_log(), vec![request("git"), request("hg")]);
    assert!(!p.blocking(&request("git")));
}
