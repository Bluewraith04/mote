//! The system source of the `random` library.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const eintr: i64 = -4;

extern "bcrypt" fn BCryptGenRandom(hAlgorithm: ?*anyopaque, pbBuffer: [*]u8, cbBuffer: u32, dwFlags: u32) callconv(.winapi) u32;

/// Fills the buffer with bytes from the system's source; gives 0 or the negated errno.
export fn random_fill(buffer: [*]u8, length: i64) i64 {
    const total: usize = @intCast(@max(length, 0));
    if (is_windows) {
        if (total == 0) return 0;
        var done: usize = 0;
        while (done < total) {
            const chunk: u32 = @intCast(@min(total - done, std.math.maxInt(u32)));
            // BCRYPT_USE_SYSTEM_PREFERRED_RNG = 0x00000002
            const status = BCryptGenRandom(null, buffer + done, chunk, 2);
            if (status != 0) return -22;
            done += chunk;
        }
        return 0;
    } else {
        var done: usize = 0;
        while (done < total) {
            const rc: i64 = @as(isize, @bitCast(linux.syscall3(.getrandom, @intFromPtr(buffer) + done, total - done, 0)));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            done += @intCast(rc);
        }
        return 0;
    }
}
