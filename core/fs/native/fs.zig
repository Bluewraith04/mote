//! The file system natives of the `fs` library: each gives a result or the negated errno.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const eintr: i64 = -@as(i64, @intFromEnum(linux.E.INTR));
const at_fdcwd: usize = @bitCast(@as(isize, -100));
const o_cloexec: usize = 0o2000000;
const o_directory: usize = 0o200000;

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn slice(pointer: [*]u8, length: i64, count: i64) []u8 {
    return pointer[0..@intCast(@max(@min(count, length), 0))];
}

/// A descriptor as a system call argument; a negative one is a bad descriptor to the kernel.
fn fd_of(fd: i64) usize {
    return @bitCast(fd);
}

export fn fs_open(path: [*:0]const u8, flags: i64, mode: i64) i64 {
    return result(linux.syscall4(.openat, at_fdcwd, @intFromPtr(path), @as(usize, @intCast(flags)) | o_cloexec, @intCast(mode)));
}

export fn fs_close(fd: i64) i64 {
    return result(linux.syscall1(.close, fd_of(fd)));
}

export fn fs_read(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const into = slice(buffer, length, count);
    while (true) {
        const rc = result(linux.syscall3(.read, fd_of(fd), @intFromPtr(into.ptr), into.len));
        if (rc != eintr) return rc;
    }
}

/// Writes the first `count` bytes, retrying a short write.
export fn fs_write(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const all = slice(buffer, length, count);
    var done: usize = 0;
    while (done < all.len) {
        const rc = result(linux.syscall3(.write, fd_of(fd), @intFromPtr(all.ptr) + done, all.len - done));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        done += @intCast(rc);
    }
    return @intCast(done);
}

export fn fs_seek(fd: i64, offset: i64, whence: i64) i64 {
    return result(linux.syscall3(.lseek, fd_of(fd), @bitCast(offset), @intCast(whence)));
}

export fn fs_truncate(fd: i64, size: i64) i64 {
    return result(linux.syscall2(.ftruncate, fd_of(fd), @bitCast(size)));
}

export fn fs_mkdir(path: [*:0]const u8, mode: i64) i64 {
    return result(linux.syscall3(.mkdirat, at_fdcwd, @intFromPtr(path), @intCast(mode)));
}

export fn fs_rmdir(path: [*:0]const u8) i64 {
    return result(linux.syscall3(.unlinkat, at_fdcwd, @intFromPtr(path), 0x200));
}

export fn fs_unlink(path: [*:0]const u8) i64 {
    return result(linux.syscall3(.unlinkat, at_fdcwd, @intFromPtr(path), 0));
}

export fn fs_rename(from: [*:0]const u8, to: [*:0]const u8) i64 {
    return result(linux.syscall4(.renameat, at_fdcwd, @intFromPtr(from), at_fdcwd, @intFromPtr(to)));
}

/// The kind, size, modification time in nanoseconds and permission bits of a path or a descriptor, in `out`.
fn fill(out: [*]i64, length: i64, stat: *const linux.Statx) i64 {
    if (length < 4) return -@as(i64, @intFromEnum(linux.E.INVAL));
    const format = stat.mode & 0o170000;
    out[0] = switch (format) {
        0o100000 => 1,
        0o040000 => 2,
        0o120000 => 3,
        else => 4,
    };
    out[1] = @intCast(stat.size);
    out[2] = @as(i64, stat.mtime.sec) * 1_000_000_000 + @as(i64, stat.mtime.nsec);
    out[3] = stat.mode & 0o7777;
    return 0;
}

/// `follow` is not 0 to look through a symbolic link.
export fn fs_stat(path: [*:0]const u8, out: [*]i64, length: i64, follow: i64) i64 {
    var stat: linux.Statx = undefined;
    const flags: usize = if (follow != 0) 0 else 0x100;
    const rc = result(linux.syscall5(.statx, at_fdcwd, @intFromPtr(path), flags, 0x7ff, @intFromPtr(&stat)));
    if (rc < 0) return rc;
    return fill(out, length, &stat);
}

export fn fs_fstat(fd: i64, out: [*]i64, length: i64) i64 {
    var stat: linux.Statx = undefined;
    const empty: [*:0]const u8 = "";
    const rc = result(linux.syscall5(.statx, fd_of(fd), @intFromPtr(empty), 0x1000, 0x7ff, @intFromPtr(&stat)));
    if (rc < 0) return rc;
    return fill(out, length, &stat);
}

/// Writes the names in a directory, each ended by a NUL, into the buffer; gives the size they need, which is more than `length` when they did not fit.
export fn fs_list(path: [*:0]const u8, buffer: [*]u8, length: i64) i64 {
    const fd = result(linux.syscall4(.openat, at_fdcwd, @intFromPtr(path), o_directory | o_cloexec, 0));
    if (fd < 0) return fd;
    defer _ = linux.syscall1(.close, fd_of(fd));
    var chunk: [4096]u8 align(8) = undefined;
    var needed: usize = 0;
    const room: usize = @intCast(@max(length, 0));
    while (true) {
        const got = result(linux.syscall3(.getdents64, fd_of(fd), @intFromPtr(&chunk), chunk.len));
        if (got < 0) return got;
        if (got == 0) break;
        var at: usize = 0;
        while (at < got) {
            const reclen = std.mem.readInt(u16, chunk[at + 16 ..][0..2], .little);
            const name: [*:0]const u8 = @ptrCast(chunk[at + 19 ..].ptr);
            const text = std.mem.span(name);
            at += reclen;
            if (std.mem.eql(u8, text, ".") or std.mem.eql(u8, text, "..")) continue;
            if (needed + text.len + 1 <= room) {
                @memcpy(buffer[needed..][0..text.len], text);
                buffer[needed + text.len] = 0;
            }
            needed += text.len + 1;
        }
    }
    return @intCast(needed);
}
