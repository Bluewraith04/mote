//! The terminal functions of the `term` library: each gives a count or the negated errno.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const eintr: i64 = -4;
const at_fdcwd: usize = @bitCast(@as(isize, -100));
const tcgets: usize = 0x5401;
const tcsets: usize = 0x5402;
const tiocgwinsz: usize = 0x5413;
const tiocswinsz: usize = 0x5414;
const tiocgptn: usize = 0x80045430;
const tiocsptlck: usize = 0x40045431;

/// How the end of the input is told from a timeout.
const end_of_input: i64 = -4096;

const win = struct {
    const HANDLE = ?*anyopaque;
    const INVALID_HANDLE_VALUE: HANDLE = @ptrFromInt(std.math.maxInt(usize));
    const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    const WAIT_OBJECT_0: u32 = 0;

    const Coord = extern struct { x: i16, y: i16 };
    const SmallRect = extern struct { left: i16, top: i16, right: i16, bottom: i16 };
    const ScreenInfo = extern struct {
        size: Coord,
        cursor: Coord,
        attrs: u16,
        window: SmallRect,
        max_window: Coord,
    };

    extern "kernel32" fn GetLastError() callconv(.winapi) u32;
    extern "kernel32" fn GetStdHandle(nStdHandle: i32) callconv(.winapi) HANDLE;
    extern "kernel32" fn GetConsoleMode(hConsoleHandle: HANDLE, lpMode: *u32) callconv(.winapi) i32;
    extern "kernel32" fn SetConsoleMode(hConsoleHandle: HANDLE, dwMode: u32) callconv(.winapi) i32;
    extern "kernel32" fn GetConsoleScreenBufferInfo(hConsoleOutput: HANDLE, lpConsoleScreenBufferInfo: *ScreenInfo) callconv(.winapi) i32;
    extern "kernel32" fn CreatePipe(hReadPipe: *HANDLE, hWritePipe: *HANDLE, lpPipeAttributes: ?*anyopaque, nSize: u32) callconv(.winapi) i32;
    extern "kernel32" fn PeekNamedPipe(hNamedPipe: HANDLE, lpBuffer: ?[*]u8, nBufferSize: u32, lpBytesRead: ?*u32, lpTotalBytesAvail: ?*u32, lpBytesLeftThisMessage: ?*u32) callconv(.winapi) i32;
    extern "kernel32" fn ReadFile(hFile: HANDLE, lpBuffer: [*]u8, nNumberOfBytesToRead: u32, lpNumberOfBytesRead: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn WriteFile(hFile: HANDLE, lpBuffer: [*]const u8, nNumberOfBytesToWrite: u32, lpNumberOfBytesWritten: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn CloseHandle(hObject: HANDLE) callconv(.winapi) i32;
    extern "kernel32" fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) callconv(.winapi) u32;
    extern "kernel32" fn Sleep(dwMilliseconds: u32) callconv(.winapi) void;
};

const PTY_BASE: i64 = 0x4000_0000;
const PTY_MAX = 32;

const PtySlot = struct {
    used: bool = false,
    raw: bool = false,
    rows: i64 = 24,
    cols: i64 = 80,
    m2s_read: win.HANDLE = null,
    m2s_write: win.HANDLE = null,
    s2m_read: win.HANDLE = null,
    s2m_write: win.HANDLE = null,
    line_len: usize = 0,
    line_buf: [1024]u8 = undefined,
};

var pty_lock: std.atomic.Value(bool) = .init(false);
var pty_slots: [PTY_MAX]PtySlot = std.mem.zeroes([PTY_MAX]PtySlot);

fn lock_pty() void {
    while (pty_lock.cmpxchgWeak(false, true, .acquire, .monotonic) != null) {}
}

fn unlock_pty() void {
    pty_lock.store(false, .release);
}

fn pty_lookup(fd: i64) ?struct { slot: *PtySlot, is_master: bool } {
    if (fd < PTY_BASE or fd >= PTY_BASE + PTY_MAX * 2) return null;
    const offset: usize = @intCast(fd - PTY_BASE);
    const slot = &pty_slots[offset / 2];
    if (!slot.used) return null;
    return .{ .slot = slot, .is_master = (offset % 2) == 0 };
}

fn win_handle_of(fd: i64) ?win.HANDLE {
    if (fd < 0) return null;
    if (fd == 0) return win.GetStdHandle(-10);
    if (fd == 1) return win.GetStdHandle(-11);
    if (fd == 2) return win.GetStdHandle(-12);
    const ptr: *anyopaque = @ptrFromInt(@as(usize, @intCast(fd)));
    return ptr;
}

fn write_all_win(h: win.HANDLE, bytes: []const u8) i64 {
    var done: usize = 0;
    while (done < bytes.len) {
        const chunk: u32 = @intCast(@min(bytes.len - done, std.math.maxInt(u32)));
        var wrote: u32 = 0;
        if (win.WriteFile(h, bytes.ptr + done, chunk, &wrote, null) == 0) return -5;
        done += wrote;
    }
    return @intCast(done);
}

fn read_pipe_win(h: win.HANDLE, buffer: [*]u8, length: i64, timeout_ms: i64) i64 {
    const cap: u32 = @intCast(@min(@max(length, 0), std.math.maxInt(u32)));
    var elapsed: i64 = 0;
    while (true) {
        var avail: u32 = 0;
        if (win.PeekNamedPipe(h, null, 0, null, &avail, null) == 0) {
            const err = win.GetLastError();
            if (err == 109) return end_of_input;
            if (err == 6) return -9;
            return -5;
        }
        if (avail > 0) {
            var got: u32 = 0;
            if (win.ReadFile(h, buffer, cap, &got, null) == 0) {
                const err = win.GetLastError();
                if (err == 109) return end_of_input;
                return -5;
            }
            if (got == 0) return end_of_input;
            return @intCast(got);
        }
        if (timeout_ms == 0 or (timeout_ms > 0 and elapsed >= timeout_ms)) return 0;
        const step: i64 = if (timeout_ms < 0) 5 else @min(5, timeout_ms - elapsed);
        win.Sleep(@intCast(step));
        elapsed += step;
    }
}

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
    if (is_windows) {
        if (pty_lookup(fd) != null) return 1;
        const h = win_handle_of(fd) orelse return 0;
        var mode: u32 = 0;
        return if (win.GetConsoleMode(h, &mode) != 0) 1 else 0;
    } else {
        var state: Termios = undefined;
        return if (result(linux.syscall3(.ioctl, fd_of(fd), tcgets, @intFromPtr(&state))) < 0) 0 else 1;
    }
}

/// The rows, columns, and pixel width and height of the terminal in `out`.
export fn term_size(fd: i64, out: [*]i64, length: i64) i64 {
    if (length < 4) return -22;
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            lock_pty();
            defer unlock_pty();
            out[0] = found.slot.rows;
            out[1] = found.slot.cols;
            out[2] = 0;
            out[3] = 0;
            return 0;
        }
        const h = win_handle_of(fd) orelse return -25;
        var info: win.ScreenInfo = undefined;
        if (win.GetConsoleScreenBufferInfo(h, &info) == 0) return -25;
        out[0] = info.window.bottom - info.window.top + 1;
        out[1] = info.window.right - info.window.left + 1;
        out[2] = 0;
        out[3] = 0;
        return 0;
    } else {
        var size: Winsize = undefined;
        const rc = result(linux.syscall3(.ioctl, fd_of(fd), tiocgwinsz, @intFromPtr(&size)));
        if (rc < 0) return rc;
        out[0] = size.rows;
        out[1] = size.cols;
        out[2] = size.xpixel;
        out[3] = size.ypixel;
        return 0;
    }
}

export fn term_set_size(fd: i64, rows: i64, cols: i64) i64 {
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            lock_pty();
            defer unlock_pty();
            found.slot.rows = rows;
            found.slot.cols = cols;
            return 0;
        }
        return 0;
    } else {
        var size = Winsize{ .rows = @intCast(rows), .cols = @intCast(cols), .xpixel = 0, .ypixel = 0 };
        return result(linux.syscall3(.ioctl, fd_of(fd), tiocswinsz, @intFromPtr(&size)));
    }
}

/// Puts the terminal in raw mode (no echo, no line editing, no signals from keys, bytes at once) and writes the settings it had into `saved`.
export fn term_raw(fd: i64, saved: [*]u8, length: i64) i64 {
    if (length < @sizeOf(Termios)) return -22;
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            var flush_pipe: win.HANDLE = null;
            var flush_bytes: [1024]u8 = undefined;
            var flush_len: usize = 0;
            {
                lock_pty();
                defer unlock_pty();
                saved[0] = if (found.slot.raw) 1 else 0;
                found.slot.raw = true;
                if (found.slot.line_len > 0) {
                    flush_pipe = found.slot.m2s_write;
                    flush_len = found.slot.line_len;
                    @memcpy(flush_bytes[0..flush_len], found.slot.line_buf[0..flush_len]);
                    found.slot.line_len = 0;
                }
            }
            if (flush_len > 0 and flush_pipe != null) {
                _ = write_all_win(flush_pipe, flush_bytes[0..flush_len]);
            }
            return 0;
        }
        const h = win_handle_of(fd) orelse return -25;
        var mode: u32 = 0;
        if (win.GetConsoleMode(h, &mode) == 0) return -25;
        std.mem.writeInt(u32, saved[0..4], mode, .little);
        const raw_mode = (mode & ~(win.ENABLE_ECHO_INPUT | win.ENABLE_LINE_INPUT | win.ENABLE_PROCESSED_INPUT)) | win.ENABLE_VIRTUAL_TERMINAL_INPUT;
        if (win.SetConsoleMode(h, raw_mode) == 0) return -22;
        if (win_handle_of(1)) |out_h| {
            var out_mode: u32 = 0;
            if (win.GetConsoleMode(out_h, &out_mode) != 0) {
                _ = win.SetConsoleMode(out_h, out_mode | win.ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
        return 0;
    } else {
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
}

/// Gives the terminal the settings `term_raw` saved.
export fn term_restore(fd: i64, saved: [*]u8, length: i64) i64 {
    if (length < @sizeOf(Termios)) return -22;
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            lock_pty();
            defer unlock_pty();
            found.slot.raw = saved[0] != 0;
            return 0;
        }
        const h = win_handle_of(fd) orelse return -25;
        const mode = std.mem.readInt(u32, saved[0..4], .little);
        if (win.SetConsoleMode(h, mode) == 0) return -25;
        return 0;
    } else {
        var state: Termios = undefined;
        @memcpy(std.mem.asBytes(&state), saved[0..@sizeOf(Termios)]);
        return result(linux.syscall3(.ioctl, fd_of(fd), tcsets, @intFromPtr(&state)));
    }
}

/// Waits up to `timeout_ms` (forever when negative) for bytes and reads up to the buffer's length: the count, 0 on a timeout, or -4096 at the end of the input.
export fn term_read(fd: i64, buffer: [*]u8, length: i64, timeout_ms: i64) i64 {
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            const pipe_h = if (found.is_master) found.slot.s2m_read else found.slot.m2s_read;
            return read_pipe_win(pipe_h, buffer, length, timeout_ms);
        }
        const h = win_handle_of(fd) orelse return -9;
        var mode: u32 = 0;
        if (win.GetConsoleMode(h, &mode) == 0) return read_pipe_win(h, buffer, length, timeout_ms);
        const ms: u32 = if (timeout_ms < 0) 0xFFFFFFFF else @intCast(@min(timeout_ms, 0x7FFFFFFF));
        const rc = win.WaitForSingleObject(h, ms);
        if (rc != win.WAIT_OBJECT_0) return 0;
        var got: u32 = 0;
        const want: u32 = @intCast(@min(@max(length, 0), std.math.maxInt(u32)));
        if (win.ReadFile(h, buffer, want, &got, null) == 0) return end_of_input;
        if (got == 0) return end_of_input;
        return @intCast(got);
    } else {
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
}

/// Writes the first `count` bytes, retrying a short write.
export fn term_write(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const total: usize = @intCast(@max(@min(count, length), 0));
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            const bytes = buffer[0..total];
            if (found.is_master) {
                var to_send: [1024]u8 = undefined;
                var send_len: usize = 0;
                var pipe_h: win.HANDLE = null;
                var direct = false;
                {
                    lock_pty();
                    defer unlock_pty();
                    pipe_h = found.slot.m2s_write;
                    if (found.slot.raw) {
                        direct = true;
                    } else {
                        for (bytes) |ch| {
                            if (found.slot.line_len < found.slot.line_buf.len) {
                                found.slot.line_buf[found.slot.line_len] = ch;
                                found.slot.line_len += 1;
                            }
                            if (ch == '\n' or ch == '\r' or found.slot.line_len == found.slot.line_buf.len) {
                                @memcpy(to_send[0..found.slot.line_len], found.slot.line_buf[0..found.slot.line_len]);
                                send_len = found.slot.line_len;
                                found.slot.line_len = 0;
                            }
                        }
                    }
                }
                if (direct) return write_all_win(pipe_h, bytes);
                if (send_len > 0) {
                    const rc = write_all_win(pipe_h, to_send[0..send_len]);
                    if (rc < 0) return rc;
                }
                return @intCast(total);
            } else {
                var pipe_h: win.HANDLE = null;
                var raw = false;
                {
                    lock_pty();
                    defer unlock_pty();
                    pipe_h = found.slot.s2m_write;
                    raw = found.slot.raw;
                }
                if (raw) return write_all_win(pipe_h, bytes);
                var start: usize = 0;
                for (bytes, 0..) |ch, idx| {
                    if (ch == '\n') {
                        if (idx > start) {
                            const r1 = write_all_win(pipe_h, bytes[start..idx]);
                            if (r1 < 0) return r1;
                        }
                        const crlf = "\r\n";
                        const r2 = write_all_win(pipe_h, crlf);
                        if (r2 < 0) return r2;
                        start = idx + 1;
                    }
                }
                if (start < bytes.len) {
                    const r3 = write_all_win(pipe_h, bytes[start..]);
                    if (r3 < 0) return r3;
                }
                return @intCast(total);
            }
        }
        const h = win_handle_of(fd) orelse return -9;
        return write_all_win(h, buffer[0..total]);
    } else {
        var done: usize = 0;
        while (done < total) {
            const rc = result(linux.syscall3(.write, fd_of(fd), @intFromPtr(buffer) + done, total - done));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            done += @intCast(rc);
        }
        return @intCast(done);
    }
}

export fn term_close(fd: i64) i64 {
    if (is_windows) {
        if (pty_lookup(fd)) |found| {
            var to_close: [2]win.HANDLE = .{ null, null };
            {
                lock_pty();
                defer unlock_pty();
                if (found.is_master) {
                    to_close[0] = found.slot.m2s_write;
                    to_close[1] = found.slot.s2m_read;
                    found.slot.m2s_write = null;
                    found.slot.s2m_read = null;
                } else {
                    to_close[0] = found.slot.s2m_write;
                    to_close[1] = found.slot.m2s_read;
                    found.slot.s2m_write = null;
                    found.slot.m2s_read = null;
                }
                if (found.slot.m2s_write == null and found.slot.s2m_read == null and found.slot.s2m_write == null and found.slot.m2s_read == null) {
                    found.slot.used = false;
                }
            }
            for (to_close) |h| {
                if (h != null) _ = win.CloseHandle(h);
            }
            return 0;
        }
        const h = win_handle_of(fd) orelse return -9;
        if (win.CloseHandle(h) == 0) return -9;
        return 0;
    } else {
        return result(linux.syscall1(.close, fd_of(fd)));
    }
}

/// Opens a pseudo-terminal pair: the master descriptor and the slave descriptor in `out`.
export fn term_pty(out: [*]i64, length: i64) i64 {
    if (length < 2) return -22;
    if (is_windows) {
        var m2s_r: win.HANDLE = null;
        var m2s_w: win.HANDLE = null;
        var s2m_r: win.HANDLE = null;
        var s2m_w: win.HANDLE = null;
        if (win.CreatePipe(&m2s_r, &m2s_w, null, 0) == 0) return -5;
        if (win.CreatePipe(&s2m_r, &s2m_w, null, 0) == 0) {
            _ = win.CloseHandle(m2s_r);
            _ = win.CloseHandle(m2s_w);
            return -5;
        }
        lock_pty();
        defer unlock_pty();
        for (&pty_slots, 0..) |*slot, idx| {
            if (!slot.used) {
                slot.* = .{
                    .used = true,
                    .raw = false,
                    .rows = 24,
                    .cols = 80,
                    .m2s_read = m2s_r,
                    .m2s_write = m2s_w,
                    .s2m_read = s2m_r,
                    .s2m_write = s2m_w,
                    .line_len = 0,
                };
                out[0] = PTY_BASE + @as(i64, @intCast(idx * 2));
                out[1] = out[0] + 1;
                return 0;
            }
        }
        _ = win.CloseHandle(m2s_r);
        _ = win.CloseHandle(m2s_w);
        _ = win.CloseHandle(s2m_r);
        _ = win.CloseHandle(s2m_w);
        return -24;
    } else {
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
}
