//! The terminal natives of the `term` library: each gives a count or the negated errno.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const eintr: i64 = -@as(i64, @intFromEnum(linux.E.INTR));
const at_fdcwd: usize = @bitCast(@as(isize, -100));
const tcgets: usize = 0x5401;
const tcsets: usize = 0x5402;
const tiocgwinsz: usize = 0x5413;
const tiocswinsz: usize = 0x5414;
const tiocgptn: usize = 0x80045430;
const tiocsptlck: usize = 0x40045431;

/// How the end of the input is told from a timeout.
const end_of_input: i64 = -4096;

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn fd_of(fd: i64) usize {
    return @bitCast(fd);
}

/// The kernel's `struct termios`: four flag words, the line discipline and the control characters.
const Termios = extern struct {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [19]u8,
};

const Winsize = extern struct {
    rows: u16,
    cols: u16,
    xpixel: u16,
    ypixel: u16,
};

export fn term_is_tty(fd: i64) i64 {
    var state: Termios = undefined;
    return if (result(linux.syscall3(.ioctl, fd_of(fd), tcgets, @intFromPtr(&state))) < 0) 0 else 1;
}

/// The rows, columns, and pixel width and height of the terminal in `out`.
export fn term_size(fd: i64, out: [*]i64, length: i64) i64 {
    if (length < 4) return -@as(i64, @intFromEnum(linux.E.INVAL));
    var size: Winsize = undefined;
    const rc = result(linux.syscall3(.ioctl, fd_of(fd), tiocgwinsz, @intFromPtr(&size)));
    if (rc < 0) return rc;
    out[0] = size.rows;
    out[1] = size.cols;
    out[2] = size.xpixel;
    out[3] = size.ypixel;
    return 0;
}

export fn term_set_size(fd: i64, rows: i64, cols: i64) i64 {
    var size = Winsize{ .rows = @intCast(rows), .cols = @intCast(cols), .xpixel = 0, .ypixel = 0 };
    return result(linux.syscall3(.ioctl, fd_of(fd), tiocswinsz, @intFromPtr(&size)));
}

/// Puts the terminal in raw mode (no echo, no line editing, no signals from keys, bytes at once) and writes the settings it had into `saved`.
export fn term_raw(fd: i64, saved: [*]u8, length: i64) i64 {
    if (length < @sizeOf(Termios)) return -@as(i64, @intFromEnum(linux.E.INVAL));
    var state: Termios = undefined;
    const got = result(linux.syscall3(.ioctl, fd_of(fd), tcgets, @intFromPtr(&state)));
    if (got < 0) return got;
    @memcpy(saved[0..@sizeOf(Termios)], std.mem.asBytes(&state));
    // IGNBRK BRKINT PARMRK ISTRIP INLCR IGNCR ICRNL IXON
    state.iflag &= ~@as(u32, 0o1 | 0o2 | 0o10 | 0o40 | 0o100 | 0o200 | 0o400 | 0o2000);
    // OPOST
    state.oflag &= ~@as(u32, 0o1);
    // CSIZE PARENB, then CS8
    state.cflag &= ~@as(u32, 0o60 | 0o400);
    state.cflag |= 0o60;
    // ECHO ECHONL ICANON ISIG IEXTEN
    state.lflag &= ~@as(u32, 0o10 | 0o100 | 0o2 | 0o1 | 0o100000);
    // VTIME 0 and VMIN 1
    state.cc[5] = 0;
    state.cc[6] = 1;
    return result(linux.syscall3(.ioctl, fd_of(fd), tcsets, @intFromPtr(&state)));
}

/// Gives the terminal the settings `term_raw` saved.
export fn term_restore(fd: i64, saved: [*]u8, length: i64) i64 {
    if (length < @sizeOf(Termios)) return -@as(i64, @intFromEnum(linux.E.INVAL));
    var state: Termios = undefined;
    @memcpy(std.mem.asBytes(&state), saved[0..@sizeOf(Termios)]);
    return result(linux.syscall3(.ioctl, fd_of(fd), tcsets, @intFromPtr(&state)));
}

/// Waits up to `timeout_ms` (forever when negative) for bytes and reads up to the buffer's length: the count, 0 on a timeout, or -4096 at the end of the input.
export fn term_read(fd: i64, buffer: [*]u8, length: i64, timeout_ms: i64) i64 {
    var entry = linux.pollfd{ .fd = @intCast(fd), .events = 1, .revents = 0 };
    while (true) {
        const waited = result(linux.syscall3(.poll, @intFromPtr(&entry), 1, @bitCast(@as(isize, @intCast(timeout_ms)))));
        if (waited == eintr) continue;
        if (waited < 0) return waited;
        if (waited == 0) return 0;
        break;
    }
    while (true) {
        const rc = result(linux.syscall3(.read, fd_of(fd), @intFromPtr(buffer), @intCast(@max(length, 0))));
        if (rc == eintr) continue;
        if (rc == 0) return end_of_input;
        return rc;
    }
}

/// Writes the first `count` bytes, retrying a short write.
export fn term_write(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const total: usize = @intCast(@max(@min(count, length), 0));
    var done: usize = 0;
    while (done < total) {
        const rc = result(linux.syscall3(.write, fd_of(fd), @intFromPtr(buffer) + done, total - done));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        done += @intCast(rc);
    }
    return @intCast(done);
}

export fn term_close(fd: i64) i64 {
    return result(linux.syscall1(.close, fd_of(fd)));
}

/// Opens a pseudo-terminal pair: the master descriptor and the slave descriptor in `out`.
export fn term_pty(out: [*]i64, length: i64) i64 {
    if (length < 2) return -@as(i64, @intFromEnum(linux.E.INVAL));
    const master = result(linux.syscall4(.openat, at_fdcwd, @intFromPtr("/dev/ptmx"), 2 | 0o400 | 0o2000000, 0));
    if (master < 0) return master;
    var unlock: c_int = 0;
    var number: c_uint = 0;
    if (result(linux.syscall3(.ioctl, fd_of(master), tiocsptlck, @intFromPtr(&unlock))) < 0 or result(linux.syscall3(.ioctl, fd_of(master), tiocgptn, @intFromPtr(&number))) < 0) {
        _ = linux.syscall1(.close, fd_of(master));
        return -@as(i64, @intFromEnum(linux.E.IO));
    }
    var name: [32:0]u8 = undefined;
    const prefix = "/dev/pts/";
    @memcpy(name[0..prefix.len], prefix);
    var digits: [10]u8 = undefined;
    var count: usize = 0;
    var rest = number;
    while (true) {
        digits[count] = '0' + @as(u8, @intCast(rest % 10));
        count += 1;
        rest /= 10;
        if (rest == 0) break;
    }
    for (0..count) |at| name[prefix.len + at] = digits[count - 1 - at];
    name[prefix.len + count] = 0;
    const slave = result(linux.syscall4(.openat, at_fdcwd, @intFromPtr(&name), 2 | 0o400 | 0o2000000, 0));
    if (slave < 0) {
        _ = linux.syscall1(.close, fd_of(master));
        return slave;
    }
    out[0] = master;
    out[1] = slave;
    return 0;
}
