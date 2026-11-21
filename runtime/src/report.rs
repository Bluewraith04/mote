//! A fault report: what stopped the program, the check that stopped it and the frames above it.

use std::fmt;

use mote_abi::FaultKind;

use crate::sys;

/// Most frames a report keeps.
pub const MAX_FRAMES: usize = 64;

/// A stop of the program, as the runtime tells it to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultReport {
    pub kind: FaultKind,
    /// The kind's message, or the program's own for `Raised`.
    pub message: String,
    /// The raw id of the fault site; `None` for a fault that has a message of its own.
    pub site: Option<u32>,
    /// Where the check is written, from the fault table: the two ends of its span.
    pub span: Option<(u32, u32)>,
    /// Return addresses, innermost first: where the entry was called, then each caller up the frame-pointer chain.
    pub backtrace: Vec<usize>,
}

impl fmt::Display for FaultReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fault: {}", self.message)?;
        for pc in &self.backtrace {
            write!(f, "\n  at {pc:#x}")?;
        }
        Ok(())
    }
}

/// The return addresses along the frame-pointer chain, `pc` first and then the one in each frame record from `fp` up.
///
/// The walk ends at a null, misaligned or unreadable record, at one that does not rise, and after `MAX_FRAMES`.
pub fn backtrace(fp: usize, pc: usize) -> Vec<usize> {
    let mut frames = vec![pc];
    let mut fp = fp;
    while frames.len() < MAX_FRAMES && fp != 0 && fp.is_multiple_of(16) {
        let Some([next, ret]) = sys::read_two_words(fp) else { break };
        if ret == 0 {
            break;
        }
        frames.push(ret);
        if next <= fp {
            break;
        }
        fp = next;
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame record: the caller's frame pointer, then the return address.
    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    struct Record([usize; 2]);

    /// A chain of `records` frame records laid out from low to high addresses, ending in a null frame pointer.
    fn chain(records: usize) -> Vec<Record> {
        let mut frames = vec![Record([0; 2]); records];
        for at in 0..records {
            let next = if at + 1 < records { frames.as_ptr() as usize + (at + 1) * 16 } else { 0 };
            frames[at] = Record([next, 0x1000 + at]);
        }
        frames
    }

    #[test]
    fn entries_a_backtrace_lists_the_call_site_then_each_return_address_up_the_chain() {
        let frames = chain(3);
        assert_eq!(backtrace(frames.as_ptr() as usize, 0xaaaa), [0xaaaa, 0x1000, 0x1001, 0x1002]);
    }

    #[test]
    fn entries_a_backtrace_ends_at_a_null_unaligned_unreadable_or_falling_pointer() {
        assert_eq!(backtrace(0, 0xaaaa), [0xaaaa]);
        let frames = chain(2);
        assert_eq!(backtrace(frames.as_ptr() as usize + 8, 0xaaaa), [0xaaaa]);
        assert_eq!(backtrace(16, 0xaaaa), [0xaaaa]);
        let mut loops = chain(2);
        loops[1].0[0] = loops.as_ptr() as usize;
        assert_eq!(backtrace(loops.as_ptr() as usize, 0xaaaa), [0xaaaa, 0x1000, 0x1001]);
    }

    #[test]
    fn entries_a_backtrace_keeps_at_most_the_frame_limit() {
        let frames = chain(MAX_FRAMES * 2);
        assert_eq!(backtrace(frames.as_ptr() as usize, 1).len(), MAX_FRAMES);
    }

    #[test]
    fn entries_a_report_prints_its_message_and_one_line_for_each_frame() {
        let report = FaultReport { kind: FaultKind::DivideByZero, message: "division by zero".to_string(), site: Some(3), span: Some((4, 9)), backtrace: vec![0x10, 0x20] };
        assert_eq!(report.to_string(), "fault: division by zero\n  at 0x10\n  at 0x20");
    }
}
