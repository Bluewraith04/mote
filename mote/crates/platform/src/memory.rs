//! How much memory this process may use: physical RAM, or the container limit when that is smaller.

/// The smaller of physical memory and the container limit, in bytes; `None` when neither can be read.
pub fn available_bytes() -> Option<u64> {
    [physical_bytes(), container_bytes()].into_iter().flatten().min()
}

/// The most resident memory this process has held, in bytes; `None` where it cannot be read.
pub fn peak_rss_bytes() -> Option<u64> {
    peak_rss()
}

/// The `key` line (such as `VmHWM`) of `/proc/self/status` text, in bytes.
pub(crate) fn parse_status_bytes(text: &str, key: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.strip_prefix(key).is_some_and(|rest| rest.starts_with(':')))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(target_os = "linux")]
fn peak_rss() -> Option<u64> {
    parse_status_bytes(&std::fs::read_to_string("/proc/self/status").ok()?, "VmHWM")
}

#[cfg(target_os = "macos")]
fn peak_rss() -> Option<u64> {
    // SAFETY: `getrusage` fills the zeroed struct it is given.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    (unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0).then_some(usage.ru_maxrss as u64)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn peak_rss() -> Option<u64> {
    None
}

/// `MemTotal` from `/proc/meminfo` text, in bytes.
pub(crate) fn parse_meminfo(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// A cgroup limit file's text, in bytes; `max` and the huge "no limit" values of cgroup v1 are `None`.
pub(crate) fn parse_cgroup_limit(text: &str) -> Option<u64> {
    let bytes: u64 = text.trim().parse().ok()?;
    (bytes < (1 << 60)).then_some(bytes)
}

#[cfg(target_os = "linux")]
fn physical_bytes() -> Option<u64> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "linux")]
fn container_bytes() -> Option<u64> {
    ["/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/memory/memory.limit_in_bytes"]
        .iter()
        .find_map(|path| parse_cgroup_limit(&std::fs::read_to_string(path).ok()?))
}

#[cfg(target_os = "macos")]
fn physical_bytes() -> Option<u64> {
    let mut bytes: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: `hw.memsize` writes one `u64` into `bytes`, and `size` says so.
    let status = unsafe {
        libc::sysctlbyname(c"hw.memsize".as_ptr(), (&mut bytes as *mut u64).cast(), &mut size, std::ptr::null_mut(), 0)
    };
    (status == 0).then_some(bytes)
}

#[cfg(windows)]
fn physical_bytes() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    // SAFETY: the declaration matches the Win32 signature of `GlobalMemoryStatusEx`.
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    }
    let mut status = MemoryStatusEx {
        length: std::mem::size_of::<MemoryStatusEx>() as u32,
        memory_load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: `length` is set, and `status` is a `MEMORYSTATUSEX`.
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.total_phys)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn physical_bytes() -> Option<u64> {
    None
}

#[cfg(not(target_os = "linux"))]
fn container_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo_gives_total_in_bytes() {
        assert_eq!(parse_meminfo("MemTotal:       16384000 kB\nMemFree: 1 kB\n"), Some(16_384_000 * 1024));
        assert_eq!(parse_meminfo("MemFree: 1 kB\n"), None);
    }

    #[test]
    fn status_lines_give_bytes() {
        let text = "VmPeak:   9000 kB\nVmHWM:     512 kB\nVmHWMx: 1 kB\n";
        assert_eq!(parse_status_bytes(text, "VmHWM"), Some(512 * 1024));
        assert_eq!(parse_status_bytes(text, "VmRSS"), None);
    }

    #[test]
    fn cgroup_limits_skip_unlimited() {
        assert_eq!(parse_cgroup_limit("536870912\n"), Some(512 << 20));
        assert_eq!(parse_cgroup_limit("max\n"), None);
        assert_eq!(parse_cgroup_limit("9223372036854771712\n"), None);
    }

    #[test]
    fn this_machine_reports_some_memory() {
        if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
            assert!(available_bytes().is_some_and(|b| b >= 64 << 20));
        }
    }
}
