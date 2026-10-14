//! OS signals as an event source: a thread forwards each delivery of a watched signal to the sink.

use contracts::{EventPayload, EventSink, PlatformError, PlatformErrorKind, SourceHandle};

#[cfg(unix)]
pub(crate) fn watch(codes: &[i64], sink: EventSink) -> Result<SourceHandle, PlatformError> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM, SIGUSR1, SIGUSR2};
    use signal_hook::iterator::Signals;

    let pairs: Vec<(i32, i64)> = codes
        .iter()
        .map(|code| match code {
            0 => Ok((SIGINT, 0)),
            1 => Ok((SIGTERM, 1)),
            2 => Ok((SIGHUP, 2)),
            3 => Ok((SIGUSR1, 3)),
            4 => Ok((SIGUSR2, 4)),
            other => Err(failed(format!("unknown signal {other}"))),
        })
        .collect::<Result<_, _>>()?;
    let numbers: Vec<i32> = pairs.iter().map(|(number, _)| *number).collect();
    let mut signals = Signals::new(&numbers).map_err(|e| failed(e.to_string()))?;
    let handle = signals.handle();
    std::thread::Builder::new()
        .name("mote-signals".to_string())
        .spawn(move || {
            for number in signals.forever() {
                let Some((_, code)) = pairs.iter().find(|(n, _)| *n == number) else { continue };
                if sink.push(EventPayload::Int(*code)) == contracts::PushResult::Closed {
                    break;
                }
            }
        })
        .map_err(|e| failed(e.to_string()))?;
    Ok(SourceHandle::new(move || handle.close()))
}

#[cfg(not(unix))]
pub(crate) fn watch(_codes: &[i64], _sink: EventSink) -> Result<SourceHandle, PlatformError> {
    Err(PlatformError { kind: PlatformErrorKind::Unsupported, message: "signals are not supported on this platform".to_string() })
}

#[cfg(unix)]
fn failed(message: String) -> PlatformError {
    PlatformError { kind: PlatformErrorKind::Other, message }
}
