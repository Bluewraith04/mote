//! The signal natives of the `signal` library: a handler writes the signal number into the pipe of each subscription that watches it.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

extern fn signal(number: c_int, handler: usize) usize;

const eintr: i64 = -@as(i64, @intFromEnum(linux.E.INTR));
const o_cloexec: usize = 0o2000000;
const o_nonblock: usize = 0o4000;
const f_setfl: usize = 4;
const slots = 64;

const Slot = struct {
    used: std.atomic.Value(bool) = .init(false),
    read: std.atomic.Value(i32) = .init(-1),
    write: std.atomic.Value(i32) = .init(-1),
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
        _ = linux.syscall3(.write, fd_of(slot.write.load(.acquire)), @intFromPtr(&byte), 1);
    }
}

fn find(read: i64) ?*Slot {
    for (&table) |*slot| {
        if (slot.used.load(.acquire) and slot.read.load(.acquire) == read) return slot;
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
    var fds: [2]i32 = undefined;
    const rc = result(linux.syscall2(.pipe2, @intFromPtr(&fds), o_cloexec));
    if (rc < 0) return rc;
    _ = linux.syscall3(.fcntl, fd_of(fds[1]), f_setfl, o_nonblock);
    for (&table) |*slot| {
        if (slot.used.cmpxchgStrong(false, true, .acq_rel, .acquire) != null) continue;
        slot.read.store(fds[0], .release);
        slot.write.store(fds[1], .release);
        slot.mask.store(0, .release);
        return fds[0];
    }
    _ = linux.syscall1(.close, fd_of(fds[0]));
    _ = linux.syscall1(.close, fd_of(fds[1]));
    return -@as(i64, @intFromEnum(linux.E.MFILE));
}

/// Makes the subscription watch a signal, and installs the handler for it.
export fn signal_watch(read: i64, number: i64) i64 {
    const slot = find(read) orelse return -@as(i64, @intFromEnum(linux.E.BADF));
    if (number < 1 or number > 63) return -@as(i64, @intFromEnum(linux.E.INVAL));
    _ = slot.mask.fetchOr(@as(u64, 1) << @intCast(number), .acq_rel);
    if (signal(@intCast(number), @intFromPtr(&handler)) == std.math.maxInt(usize)) return -@as(i64, @intFromEnum(linux.E.INVAL));
    return 0;
}

/// Waits for the next signal of the subscription: its number, 0 once the subscription is stopped, or the negated errno.
export fn signal_next(read: i64) i64 {
    var byte: u8 = 0;
    while (true) {
        const rc = result(linux.syscall3(.read, fd_of(read), @intFromPtr(&byte), 1));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        if (rc == 0) return 0;
        return byte;
    }
}

/// Stops the subscription: its signals go back to their defaults unless another subscription watches them, and `signal_next` gives 0 after the signals already queued.
export fn signal_stop(read: i64) i64 {
    const slot = find(read) orelse return -@as(i64, @intFromEnum(linux.E.BADF));
    const mask = slot.mask.swap(0, .acq_rel);
    const write = slot.write.load(.acquire);
    slot.used.store(false, .release);
    var number: u6 = 1;
    while (number < 63) : (number += 1) {
        if ((mask >> number) & 1 == 1 and !watched(number)) _ = signal(number, 0);
    }
    // Closing the write end is what ends the reader, after the bytes already in the pipe.
    _ = linux.syscall1(.close, fd_of(write));
    return 0;
}

/// Closes the read end of a stopped subscription.
export fn signal_free(read: i64) i64 {
    return result(linux.syscall1(.close, fd_of(read)));
}

/// Sends a signal to a process. A signal to this process goes to the calling thread, so its handler has run when the call returns.
export fn signal_send(pid: i64, number: i64) i64 {
    const own = result(linux.syscall0(.getpid));
    if (pid == own) {
        const thread = result(linux.syscall0(.gettid));
        return result(linux.syscall3(.tgkill, fd_of(own), fd_of(thread), @intCast(number)));
    }
    return result(linux.syscall2(.kill, fd_of(pid), @intCast(number)));
}

export fn signal_pid() i64 {
    return result(linux.syscall0(.getpid));
}
