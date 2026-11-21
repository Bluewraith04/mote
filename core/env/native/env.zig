//! The environment natives of the `env` library: each gives a size or the negated errno.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

extern var environ: [*:null]?[*:0]u8;
extern fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;
extern fn unsetenv(name: [*:0]const u8) c_int;

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn room(buffer: [*]u8, length: i64) []u8 {
    return buffer[0..@intCast(@max(length, 0))];
}

/// Copies the value of a variable into the buffer and gives its length, or -1 when it is not set; a length over the buffer's means it did not fit.
export fn env_get(name: [*:0]const u8, buffer: [*]u8, length: i64) i64 {
    const wanted = std.mem.span(name);
    var at: usize = 0;
    while (environ[at]) |entry| : (at += 1) {
        const text = std.mem.span(entry);
        const split = std.mem.indexOfScalar(u8, text, '=') orelse continue;
        if (!std.mem.eql(u8, text[0..split], wanted)) continue;
        const value = text[split + 1 ..];
        const out = room(buffer, length);
        @memcpy(out[0..@min(out.len, value.len)], value[0..@min(out.len, value.len)]);
        return @intCast(value.len);
    }
    return -1;
}

export fn env_set(name: [*:0]const u8, value: [*:0]const u8) i64 {
    if (setenv(name, value, 1) != 0) return -@as(i64, @intFromEnum(linux.E.INVAL));
    return 0;
}

export fn env_remove(name: [*:0]const u8) i64 {
    if (unsetenv(name) != 0) return -@as(i64, @intFromEnum(linux.E.INVAL));
    return 0;
}

/// Writes every `NAME=value` ended by a NUL into the buffer and gives the size they need.
export fn env_list(buffer: [*]u8, length: i64) i64 {
    const out = room(buffer, length);
    var needed: usize = 0;
    var at: usize = 0;
    while (environ[at]) |entry| : (at += 1) {
        const text = std.mem.span(entry);
        if (needed + text.len + 1 <= out.len) {
            @memcpy(out[needed..][0..text.len], text);
            out[needed + text.len] = 0;
        }
        needed += text.len + 1;
    }
    return @intCast(needed);
}

/// Writes the working directory into the buffer and gives its length, which is more than the buffer's when it did not fit.
export fn env_cwd(buffer: [*]u8, length: i64) i64 {
    var scratch: [4096]u8 = undefined;
    const rc = result(linux.syscall2(.getcwd, @intFromPtr(&scratch), scratch.len));
    if (rc < 0) return rc;
    const size: usize = @intCast(rc - 1);
    const out = room(buffer, length);
    @memcpy(out[0..@min(out.len, size)], scratch[0..@min(out.len, size)]);
    return @intCast(size);
}

export fn env_chdir(path: [*:0]const u8) i64 {
    return result(linux.syscall1(.chdir, @intFromPtr(path)));
}
