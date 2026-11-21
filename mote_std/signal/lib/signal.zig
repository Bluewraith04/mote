//! The signal functions of the `signal` library: a handler writes the signal number into the pipe of each subscription that watches it.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const crt_signal = if (is_windows)
    struct {
        extern "ucrtbase" fn signal(number: c_int, handler: usize) callconv(.c) usize;
    }.signal
else
    struct {
        extern fn signal(number: c_int, handler: usize) usize;
    }.signal;

const win = struct {
    const HANDLE = ?*anyopaque;
    extern "kernel32" fn CreatePipe(hReadPipe: *HANDLE, hWritePipe: *HANDLE, lpPipeAttributes: ?*anyopaque, nSize: u32) callconv(.winapi) i32;
    extern "kernel32" fn ReadFile(hFile: HANDLE, lpBuffer: [*]u8, nNumberOfBytesToRead: u32, lpNumberOfBytesRead: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn WriteFile(hFile: HANDLE, lpBuffer: [*]const u8, nNumberOfBytesToWrite: u32, lpNumberOfBytesWritten: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn CloseHandle(hObject: HANDLE) callconv(.winapi) i32;
    extern "kernel32" fn GetCurrentProcessId() callconv(.winapi) u32;
    extern "kernel32" fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) callconv(.winapi) HANDLE;
    extern "kernel32" fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) callconv(.winapi) i32;
    extern "kernel32" fn GetLastError() callconv(.winapi) u32;
};

fn win_is_crt_signal(number: i64) bool {
    return switch (number) {
        2, 4, 8, 11, 15, 21, 22 => true,
        else => false,
    };
}

const eintr: i64 = -4;
const o_cloexec: usize = 0o2000000;
const o_nonblock: usize = 0o4000;
const f_setfl: usize = 4;
const slots = 64;

const Slot = struct {
    used: std.atomic.Value(bool) = .init(false),
    read: std.atomic.Value(usize) = .init(0),
    write: std.atomic.Value(usize) = .init(0),
    mask: std.atomic.Value(u64) = .init(0),
};

var table: [slots]Slot = std.mem.zeroes([slots]Slot);

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn fd_of(fd: i64) usize {
    return @bitCast(fd);
}

fn handler(number: c_int) callconv(.c) void {
    const byte: u8 = @intCast(number);
    for (&table) |*slot| {
        if (!slot.used.load(.acquire)) continue;
        if ((slot.mask.load(.acquire) >> @intCast(number)) & 1 == 0) continue;
        const w = slot.write.load(.acquire);
        if (is_windows) {
            var wrote: u32 = 0;
            _ = win.WriteFile(@ptrFromInt(w), (&byte)[0..1], 1, &wrote, null);
        } else {
            _ = linux.syscall3(.write, w, @intFromPtr(&byte), 1);
        }
    }
    if (is_windows and win_is_crt_signal(number)) {
        _ = crt_signal(number, @intFromPtr(&handler));
    }
}

fn find(read: i64) ?*Slot {
    if (read < 0) return null;
    const key: usize = @intCast(read);
    for (&table) |*slot| {
        if (slot.used.load(.acquire) and slot.read.load(.acquire) == key) return slot;
    }
    return null;
}

/// Whether any subscription still watches the signal.
fn watched(number: u6) bool {
    for (&table) |*slot| {
        if (slot.used.load(.acquire) and (slot.mask.load(.acquire) >> number) & 1 == 1) return true;
    }
    return false;
}

/// A new subscription that watches nothing yet. Returns the descriptor its signals are read from.
export fn signal_open() i64 {
    if (is_windows) {
        var r: win.HANDLE = null;
        var w: win.HANDLE = null;
        if (win.CreatePipe(&r, &w, null, 0) == 0) return -24;
        const ri = @intFromPtr(r);
        const wi = @intFromPtr(w);
        for (&table) |*slot| {
            if (slot.used.cmpxchgStrong(false, true, .acq_rel, .acquire) != null) continue;
            slot.read.store(ri, .release);
            slot.write.store(wi, .release);
            slot.mask.store(0, .release);
            return @intCast(ri);
        }
        _ = win.CloseHandle(r);
        _ = win.CloseHandle(w);
        return -24;
    } else {
        var fds: [2]i32 = undefined;
        const rc = result(linux.syscall2(.pipe2, @intFromPtr(&fds), o_cloexec));
        if (rc < 0) return rc;
        _ = linux.syscall3(.fcntl, fd_of(fds[1]), f_setfl, o_nonblock);
        for (&table) |*slot| {
            if (slot.used.cmpxchgStrong(false, true, .acq_rel, .acquire) != null) continue;
            slot.read.store(@intCast(fds[0]), .release);
            slot.write.store(@intCast(fds[1]), .release);
            slot.mask.store(0, .release);
            return fds[0];
        }
        _ = linux.syscall1(.close, fd_of(fds[0]));
        _ = linux.syscall1(.close, fd_of(fds[1]));
        return -24;
    }
}

/// Makes the subscription watch a signal, and installs the handler for it.
export fn signal_watch(read: i64, number: i64) i64 {
    const slot = find(read) orelse return -9;
    if (number < 1 or number > 63) return -22;
    _ = slot.mask.fetchOr(@as(u64, 1) << @intCast(number), .acq_rel);
    if (!is_windows or win_is_crt_signal(number)) {
        if (crt_signal(@intCast(number), @intFromPtr(&handler)) == std.math.maxInt(usize)) return -22;
    }
    return 0;
}

/// Waits for the next signal of the subscription: its number, 0 once the subscription is stopped, or the negated errno.
export fn signal_next(read: i64) i64 {
    var byte: u8 = 0;
    if (is_windows) {
        if (read < 0) return -9;
        var got: u32 = 0;
        const h: win.HANDLE = @ptrFromInt(@as(usize, @intCast(read)));
        if (win.ReadFile(h, (&byte)[0..1], 1, &got, null) == 0) {
            const err = win.GetLastError();
            if (err == 109) return 0; // ERROR_BROKEN_PIPE
            if (err == 6) return -9; // INVALID_HANDLE
            return -5;
        }
        if (got == 0) return 0;
        return byte;
    } else {
        while (true) {
            const rc = result(linux.syscall3(.read, fd_of(read), @intFromPtr(&byte), 1));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            if (rc == 0) return 0;
            return byte;
        }
    }
}

/// Stops the subscription: its signals go back to their defaults unless another subscription watches them, and `signal_next` gives 0 after the signals already queued.
export fn signal_stop(read: i64) i64 {
    const slot = find(read) orelse return -9;
    const mask = slot.mask.swap(0, .acq_rel);
    const write = slot.write.load(.acquire);
    slot.used.store(false, .release);
    var number: u6 = 1;
    while (number < 63) : (number += 1) {
        if ((mask >> number) & 1 == 1 and !watched(number)) {
            if (!is_windows or win_is_crt_signal(number)) _ = crt_signal(number, 0);
        }
    }
    // Closing the write end is what ends the reader, after the bytes already in the pipe.
    if (is_windows) {
        _ = win.CloseHandle(@ptrFromInt(write));
    } else {
        _ = linux.syscall1(.close, write);
    }
    return 0;
}

/// Closes the read end of a stopped subscription.
export fn signal_free(read: i64) i64 {
    if (is_windows) {
        if (read < 0) return -9;
        if (win.CloseHandle(@ptrFromInt(@as(usize, @intCast(read)))) == 0) return -9;
        return 0;
    } else {
        return result(linux.syscall1(.close, fd_of(read)));
    }
}

/// Sends a signal to a process. A signal to this process goes to the calling thread, so its handler has run when the call returns.
export fn signal_send(pid: i64, number: i64) i64 {
    if (is_windows) {
        const own: i64 = @intCast(win.GetCurrentProcessId());
        if (pid == own) {
            if (number < 1 or number > 63) return -22;
            handler(@intCast(number));
            return 0;
        }
        if (pid <= 0) return -3;
        const h = win.OpenProcess(0x0001, 0, @intCast(pid)); // PROCESS_TERMINATE
        if (h == null) {
            const err = win.GetLastError();
            return if (err == 5) -1 else -3;
        }
        defer _ = win.CloseHandle(h);
        if (number == 9 or number == 15) {
            if (win.TerminateProcess(h, 1) == 0) return -1;
            return 0;
        }
        return -1;
    } else {
        const own = result(linux.syscall0(.getpid));
        if (pid == own) {
            const thread = result(linux.syscall0(.gettid));
            return result(linux.syscall3(.tgkill, fd_of(own), fd_of(thread), @intCast(number)));
        }
        return result(linux.syscall2(.kill, fd_of(pid), @intCast(number)));
    }
}

export fn signal_pid() i64 {
    if (is_windows) {
        return @intCast(win.GetCurrentProcessId());
    } else {
        return result(linux.syscall0(.getpid));
    }
}
