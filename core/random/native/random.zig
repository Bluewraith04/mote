//! The system source of the `random` library.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const eintr: i64 = -@as(i64, @intFromEnum(linux.E.INTR));

/// Fills the buffer with bytes from the system's source; gives 0 or the negated errno.
export fn random_fill(buffer: [*]u8, length: i64) i64 {
    var done: usize = 0;
    const total: usize = @intCast(@max(length, 0));
    while (done < total) {
        const rc: i64 = @as(isize, @bitCast(linux.syscall3(.getrandom, @intFromPtr(buffer) + done, total - done, 0)));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        done += @intCast(rc);
    }
    return 0;
}
