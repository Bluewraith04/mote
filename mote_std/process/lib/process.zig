//! The process functions of the `process` library: each gives a result or the negated errno.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

extern var environ: [*:null]?[*:0]u8;
extern fn posix_spawnp(pid: *c_int, file: [*:0]const u8, actions: *anyopaque, attr: *anyopaque, argv: [*:null]const ?[*:0]const u8, envp: [*:null]const ?[*:0]const u8) c_int;
extern fn posix_spawn_file_actions_init(actions: *anyopaque) c_int;
extern fn posix_spawn_file_actions_destroy(actions: *anyopaque) c_int;
extern fn posix_spawn_file_actions_adddup2(actions: *anyopaque, fd: c_int, new_fd: c_int) c_int;
extern fn posix_spawn_file_actions_addopen(actions: *anyopaque, fd: c_int, path: [*:0]const u8, flags: c_int, mode: c_uint) c_int;
extern fn posix_spawn_file_actions_addchdir_np(actions: *anyopaque, path: [*:0]const u8) c_int;
extern fn posix_spawnattr_init(attr: *anyopaque) c_int;
extern fn posix_spawnattr_destroy(attr: *anyopaque) c_int;
extern fn posix_spawnattr_setflags(attr: *anyopaque, flags: c_short) c_int;
extern fn posix_spawnattr_setsigmask(attr: *anyopaque, set: *const anyopaque) c_int;
extern fn posix_spawnattr_setsigdefault(attr: *anyopaque, set: *const anyopaque) c_int;
extern fn sigemptyset(set: *anyopaque) c_int;
extern fn sigfillset(set: *anyopaque) c_int;
extern fn signal(number: c_int, handler: usize) usize;

const win = struct {
    const HANDLE = ?*anyopaque;
    const INVALID_HANDLE_VALUE: HANDLE = @ptrFromInt(std.math.maxInt(usize));
    const STARTF_USESTDHANDLES: u32 = 0x00000100;
    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x00000400;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
    const DUPLICATE_SAME_ACCESS: u32 = 0x00000002;
    const INFINITE: u32 = 0xFFFFFFFF;
    const WAIT_TIMEOUT: u32 = 0x00000102;
    const GENERIC_READ: u32 = 0x80000000;
    const GENERIC_WRITE: u32 = 0x40000000;
    const FILE_SHARE_ALL: u32 = 1 | 2 | 4;
    const OPEN_EXISTING: u32 = 3;

    const SecurityAttributes = extern struct {
        length: u32,
        descriptor: ?*anyopaque,
        inherit: i32,
    };

    const StartupInfoW = extern struct {
        cb: u32,
        reserved: ?[*:0]u16,
        desktop: ?[*:0]u16,
        title: ?[*:0]u16,
        x: u32,
        y: u32,
        x_size: u32,
        y_size: u32,
        x_chars: u32,
        y_chars: u32,
        fill_attr: u32,
        flags: u32,
        show_window: u16,
        cb_reserved2: u16,
        lp_reserved2: ?*u8,
        std_input: HANDLE,
        std_output: HANDLE,
        std_error: HANDLE,
    };

    const ProcessInfo = extern struct {
        process: HANDLE,
        thread: HANDLE,
        process_id: u32,
        thread_id: u32,
    };

    extern "kernel32" fn GetLastError() callconv(.winapi) u32;
    extern "kernel32" fn GetCurrentProcess() callconv(.winapi) HANDLE;
    extern "kernel32" fn GetStdHandle(nStdHandle: i32) callconv(.winapi) HANDLE;
    extern "kernel32" fn CreatePipe(hReadPipe: *HANDLE, hWritePipe: *HANDLE, lpPipeAttributes: ?*SecurityAttributes, nSize: u32) callconv(.winapi) i32;
    extern "kernel32" fn SetHandleInformation(hObject: HANDLE, dwMask: u32, dwFlags: u32) callconv(.winapi) i32;
    extern "kernel32" fn DuplicateHandle(hSourceProcessHandle: HANDLE, hSourceHandle: HANDLE, hTargetProcessHandle: HANDLE, lpTargetHandle: *HANDLE, dwDesiredAccess: u32, bInheritHandle: i32, dwOptions: u32) callconv(.winapi) i32;
    extern "kernel32" fn CreateFileW(lpFileName: [*:0]const u16, dwDesiredAccess: u32, dwShareMode: u32, lpSecurityAttributes: ?*SecurityAttributes, dwCreationDisposition: u32, dwFlagsAndAttributes: u32, hTemplateFile: HANDLE) callconv(.winapi) HANDLE;
    extern "kernel32" fn CreateProcessW(lpApplicationName: ?[*:0]const u16, lpCommandLine: ?[*:0]u16, lpProcessAttributes: ?*anyopaque, lpThreadAttributes: ?*anyopaque, bInheritHandles: i32, dwCreationFlags: u32, lpEnvironment: ?*anyopaque, lpCurrentDirectory: ?[*:0]const u16, lpStartupInfo: *StartupInfoW, lpProcessInformation: *ProcessInfo) callconv(.winapi) i32;
    extern "kernel32" fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) callconv(.winapi) u32;
    extern "kernel32" fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *u32) callconv(.winapi) i32;
    extern "kernel32" fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) callconv(.winapi) i32;
    extern "kernel32" fn ReadFile(hFile: HANDLE, lpBuffer: [*]u8, nNumberOfBytesToRead: u32, lpNumberOfBytesRead: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn WriteFile(hFile: HANDLE, lpBuffer: [*]const u8, nNumberOfBytesToWrite: u32, lpNumberOfBytesWritten: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn CloseHandle(hObject: HANDLE) callconv(.winapi) i32;
    extern "kernel32" fn MultiByteToWideChar(CodePage: u32, dwFlags: u32, lpMultiByteStr: [*]const u8, cbMultiByte: i32, lpWideCharStr: ?[*]u16, cchWideChar: i32) callconv(.winapi) i32;
};

const ProcSlot = struct {
    used: bool = false,
    pid: i64 = 0,
    handle: win.HANDLE = null,
    killed_sig: i64 = -1,
};

var proc_lock: std.atomic.Value(bool) = .init(false);
var proc_slots: [256]ProcSlot = std.mem.zeroes([256]ProcSlot);

fn lock_procs() void {
    while (proc_lock.cmpxchgWeak(false, true, .acquire, .monotonic) != null) {}
}

fn unlock_procs() void {
    proc_lock.store(false, .release);
}

fn proc_register(pid: u32, handle: win.HANDLE) bool {
    lock_procs();
    defer unlock_procs();
    for (&proc_slots) |*slot| {
        if (!slot.used) {
            slot.* = .{ .used = true, .pid = @intCast(pid), .handle = handle, .killed_sig = -1 };
            return true;
        }
    }
    return false;
}

fn win_to_utf16(alloc: std.mem.Allocator, text: []const u8) ?[:0]u16 {
    if (text.len == 0) return alloc.allocSentinel(u16, 0, 0) catch null;
    const needed = win.MultiByteToWideChar(65001, 0, text.ptr, @intCast(text.len), null, 0);
    if (needed <= 0) return null;
    const buf = alloc.allocSentinel(u16, @intCast(needed), 0) catch return null;
    const written = win.MultiByteToWideChar(65001, 0, text.ptr, @intCast(text.len), buf.ptr, needed);
    if (written <= 0) {
        alloc.free(buf);
        return null;
    }
    return buf;
}

fn win_errno(err: u32) i64 {
    return switch (err) {
        2, 3, 123, 161 => -2, // ENOENT
        5 => -13, // EACCES
        6 => -9, // EBADF
        8, 14 => -12, // ENOMEM
        87 => -22, // EINVAL
        109, 232 => -32, // EPIPE
        193, 216 => -8, // ENOEXEC
        206 => -36, // ENAMETOOLONG
        267 => -20, // ENOTDIR
        else => -2,
    };
}

fn append_win_arg(list: *std.ArrayListUnmanaged(u8), alloc: std.mem.Allocator, arg: []const u8) bool {
    if (list.items.len > 0) list.append(alloc, ' ') catch return false;
    const needs_quotes = arg.len == 0 or std.mem.indexOfAny(u8, arg, " \t\n\x0b\"") != null;
    if (!needs_quotes) {
        list.appendSlice(alloc, arg) catch return false;
        return true;
    }
    list.append(alloc, '"') catch return false;
    var slashes: usize = 0;
    for (arg) |ch| {
        if (ch == '\\') {
            slashes += 1;
        } else if (ch == '"') {
            for (0..slashes * 2 + 1) |_| list.append(alloc, '\\') catch return false;
            list.append(alloc, '"') catch return false;
            slashes = 0;
        } else {
            for (0..slashes) |_| list.append(alloc, '\\') catch return false;
            slashes = 0;
            list.append(alloc, ch) catch return false;
        }
    }
    for (0..slashes * 2) |_| list.append(alloc, '\\') catch return false;
    list.append(alloc, '"') catch return false;
    return true;
}

const eintr: i64 = -4;
const setsigdef: c_short = 4;
const setsigmask: c_short = 8;
const sigpipe = 13;
const sig_ign: usize = 1;
const o_cloexec: usize = 0o2000000;
const o_rdwr: c_int = 2;

/// Room for glibc's `posix_spawn_file_actions_t`, `posix_spawnattr_t` and `sigset_t`, with space to spare.
const Opaque = extern struct { bytes: [512]u8 align(16) };

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn fd_of(fd: i64) usize {
    return @bitCast(fd);
}

/// The strings of a block that ends each one with a NUL, as an argument vector with a null at the end.
fn vector(block: []const u8, count: usize) ?[*:null]const ?[*:0]const u8 {
    const items = std.heap.page_allocator.alloc(?[*:0]const u8, count + 1) catch return null;
    var at: usize = 0;
    var start: usize = 0;
    for (block, 0..) |byte, index| {
        if (byte != 0) continue;
        items[at] = @ptrCast(block[start..index :0].ptr);
        at += 1;
        start = index + 1;
    }
    items[at] = null;
    return @ptrCast(items.ptr);
}

fn count_strings(block: []const u8) usize {
    var count: usize = 0;
    for (block) |byte| {
        if (byte == 0) count += 1;
    }
    return count;
}

fn release(items: [*:null]const ?[*:0]const u8, count: usize) void {
    const slice: []const ?[*:0]const u8 = @as([*]const ?[*:0]const u8, @ptrCast(items))[0 .. count + 1];
    std.heap.page_allocator.free(slice);
}

fn pipe(fds: *[2]i32) i64 {
    return result(linux.syscall2(.pipe2, @intFromPtr(fds), o_cloexec));
}

fn close_fd(fd: i32) void {
    _ = linux.syscall1(.close, fd_of(fd));
}

fn spawn_windows(arg_block: []const u8, env_block: []const u8, env_inherit: i64, cwd: [*:0]const u8, modes: i64, fds: [*]u8) i64 {
    const alloc = std.heap.page_allocator;
    var cmdline: std.ArrayListUnmanaged(u8) = .empty;
    defer cmdline.deinit(alloc);
    var start: usize = 0;
    for (arg_block, 0..) |byte, index| {
        if (byte != 0) continue;
        if (!append_win_arg(&cmdline, alloc, arg_block[start..index])) return -12;
        start = index + 1;
    }
    const wcmd = win_to_utf16(alloc, cmdline.items) orelse return -12;
    defer alloc.free(wcmd);

    var wcwd_buf: ?[:0]u16 = null;
    defer if (wcwd_buf) |b| alloc.free(b);
    const cwd_slice = std.mem.span(cwd);
    if (cwd_slice.len > 0) {
        wcwd_buf = win_to_utf16(alloc, cwd_slice) orelse return -12;
    }

    var wenv_buf: ?[]u16 = null;
    defer if (wenv_buf) |b| alloc.free(b);
    if (env_inherit == 0) {
        var flat: std.ArrayListUnmanaged(u16) = .empty;
        defer flat.deinit(alloc);
        var e_start: usize = 0;
        for (env_block, 0..) |byte, index| {
            if (byte != 0) continue;
            const item = env_block[e_start..index];
            const wide = win_to_utf16(alloc, item) orelse return -12;
            defer alloc.free(wide);
            flat.appendSlice(alloc, wide[0 .. wide.len + 1]) catch return -12;
            e_start = index + 1;
        }
        flat.append(alloc, 0) catch return -12;
        wenv_buf = flat.toOwnedSlice(alloc) catch return -12;
    }

    var sa = win.SecurityAttributes{ .length = @sizeOf(win.SecurityAttributes), .descriptor = null, .inherit = 1 };
    var parent_ends: [3]win.HANDLE = .{ null, null, null };
    var child_ends: [3]win.HANDLE = .{ null, null, null };
    var owns_child: [3]bool = .{ false, false, false };

    const nul_w = [_:0]u16{ 'N', 'U', 'L' };
    const std_ids = [_]i32{ -10, -11, -12 };
    var failed: i64 = 0;
    for (0..3) |stream| {
        const mode = (modes >> @intCast(stream * 2)) & 3;
        switch (mode) {
            1 => {
                var r: win.HANDLE = null;
                var w: win.HANDLE = null;
                if (win.CreatePipe(&r, &w, &sa, 0) == 0) {
                    failed = win_errno(win.GetLastError());
                    break;
                }
                const c_end = if (stream == 0) r else w;
                const p_end = if (stream == 0) w else r;
                _ = win.SetHandleInformation(p_end, win.HANDLE_FLAG_INHERIT, 0);
                parent_ends[stream] = p_end;
                child_ends[stream] = c_end;
                owns_child[stream] = true;
            },
            2 => {
                const access = if (stream == 0) win.GENERIC_READ else win.GENERIC_WRITE;
                const h = win.CreateFileW(&nul_w, access, win.FILE_SHARE_ALL, &sa, win.OPEN_EXISTING, 0x80, null);
                if (h == win.INVALID_HANDLE_VALUE) {
                    failed = win_errno(win.GetLastError());
                    break;
                }
                child_ends[stream] = h;
                owns_child[stream] = true;
            },
            3 => {
                var dup: win.HANDLE = null;
                if (win.DuplicateHandle(win.GetCurrentProcess(), child_ends[1], win.GetCurrentProcess(), &dup, 0, 1, win.DUPLICATE_SAME_ACCESS) == 0) {
                    failed = win_errno(win.GetLastError());
                    break;
                }
                child_ends[2] = dup;
                owns_child[2] = true;
            },
            else => {
                child_ends[stream] = win.GetStdHandle(std_ids[stream]);
            },
        }
    }

    var pi = std.mem.zeroes(win.ProcessInfo);
    if (failed == 0) {
        var si = std.mem.zeroes(win.StartupInfoW);
        si.cb = @sizeOf(win.StartupInfoW);
        si.flags = win.STARTF_USESTDHANDLES;
        si.std_input = child_ends[0];
        si.std_output = child_ends[1];
        si.std_error = child_ends[2];
        const env_ptr: ?*anyopaque = if (wenv_buf) |b| @ptrCast(b.ptr) else null;
        const cwd_ptr: ?[*:0]const u16 = if (wcwd_buf) |b| b.ptr else null;
        var flags: u32 = win.CREATE_NO_WINDOW;
        if (env_inherit == 0) flags |= win.CREATE_UNICODE_ENVIRONMENT;
        if (win.CreateProcessW(null, wcmd.ptr, null, null, 1, flags, env_ptr, cwd_ptr, &si, &pi) == 0) {
            failed = win_errno(win.GetLastError());
        } else {
            _ = win.CloseHandle(pi.thread);
            if (!proc_register(pi.process_id, pi.process)) {
                _ = win.TerminateProcess(pi.process, 1);
                _ = win.CloseHandle(pi.process);
                failed = -24;
            }
        }
    }

    const out = fds[0..12];
    for (0..3) |index| {
        if (owns_child[index] and child_ends[index] != null) _ = win.CloseHandle(child_ends[index]);
        if (failed != 0 and parent_ends[index] != null) {
            _ = win.CloseHandle(parent_ends[index]);
            parent_ends[index] = null;
        }
        const kept: i32 = if (parent_ends[index]) |h| @intCast(@intFromPtr(h)) else -1;
        std.mem.writeInt(i32, out[index * 4 ..][0..4], kept, .little);
    }
    if (failed != 0) return failed;
    return @intCast(pi.process_id);
}

/// Starts a program. `modes` holds the stream of each of standard input, output and error in two bits (0 inherited, 1 a pipe, 2 the null device, 3 for error only: standard output). `fds` receives three 32-bit little-endian descriptors, the parent's end of each pipe or -1. `env_inherit` is 1 to pass this process's environment, else the NUL-ended strings of `envp`. Returns the process id.
export fn process_spawn(program: [*:0]const u8, argv: [*]const u8, argv_length: i64, envp: [*]const u8, envp_length: i64, env_inherit: i64, cwd: [*:0]const u8, modes: i64, fds: [*]u8, fds_length: i64) i64 {
    _ = fds_length;
    const arg_block = argv[0..@intCast(argv_length)];
    const env_block = envp[0..@intCast(envp_length)];
    if (is_windows) {
        return spawn_windows(arg_block, env_block, env_inherit, cwd, modes, fds);
    }
    const arg_count = count_strings(arg_block);
    const env_count = count_strings(env_block);
    const args = vector(arg_block, arg_count) orelse return -@as(i64, @intFromEnum(linux.E.NOMEM));
    defer release(args, arg_count);
    var envs: [*:null]const ?[*:0]const u8 = @ptrCast(&environ[0]);
    if (env_inherit == 0) {
        envs = vector(env_block, env_count) orelse return -@as(i64, @intFromEnum(linux.E.NOMEM));
    }
    defer if (env_inherit == 0) release(envs, env_count);

    _ = signal(sigpipe, sig_ign);

    var pipes: [3][2]i32 = .{ .{ -1, -1 }, .{ -1, -1 }, .{ -1, -1 } };
    var actions: Opaque = undefined;
    var attr: Opaque = undefined;
    var empty: Opaque = undefined;
    var full: Opaque = undefined;
    _ = posix_spawn_file_actions_init(&actions);
    defer _ = posix_spawn_file_actions_destroy(&actions);
    _ = posix_spawnattr_init(&attr);
    defer _ = posix_spawnattr_destroy(&attr);
    _ = sigemptyset(&empty);
    _ = sigfillset(&full);
    _ = posix_spawnattr_setsigmask(&attr, &empty);
    _ = posix_spawnattr_setsigdefault(&attr, &full);
    _ = posix_spawnattr_setflags(&attr, setsigdef | setsigmask);

    var failed: i64 = 0;
    var stream: usize = 0;
    while (stream < 3 and failed == 0) : (stream += 1) {
        const mode = (modes >> @intCast(stream * 2)) & 3;
        switch (mode) {
            1 => {
                const rc = pipe(&pipes[stream]);
                if (rc < 0) {
                    failed = rc;
                    break;
                }
                // The child end is the one it reads for standard input and writes otherwise.
                const child: i32 = if (stream == 0) pipes[stream][0] else pipes[stream][1];
                _ = posix_spawn_file_actions_adddup2(&actions, child, @intCast(stream));
            },
            2 => _ = posix_spawn_file_actions_addopen(&actions, @intCast(stream), "/dev/null", if (stream == 0) 0 else o_rdwr, 0),
            3 => _ = posix_spawn_file_actions_adddup2(&actions, 1, 2),
            else => {},
        }
    }
    if (failed == 0 and cwd[0] != 0) {
        const rc = posix_spawn_file_actions_addchdir_np(&actions, cwd);
        if (rc != 0) failed = -@as(i64, rc);
    }

    var pid: c_int = 0;
    if (failed == 0) {
        const rc = posix_spawnp(&pid, program, &actions, &attr, args, envs);
        if (rc != 0) failed = -@as(i64, rc);
    }

    // The ends the child owns close in the parent either way.
    const out = fds[0..12];
    for (0..3) |index| {
        const child_end: i32 = if (index == 0) pipes[index][0] else pipes[index][1];
        const parent_end: i32 = if (index == 0) pipes[index][1] else pipes[index][0];
        if (child_end >= 0) close_fd(child_end);
        if (failed != 0 and parent_end >= 0) close_fd(parent_end);
        const kept: i32 = if (failed != 0) -1 else parent_end;
        std.mem.writeInt(i32, out[index * 4 ..][0..4], kept, .little);
    }
    if (failed != 0) return failed;
    return pid;
}

fn wait_win(pid: i64, blocking: bool) i64 {
    var handle: win.HANDLE = null;
    {
        lock_procs();
        defer unlock_procs();
        for (&proc_slots) |*slot| {
            if (slot.used and slot.pid == pid) {
                handle = slot.handle;
                break;
            }
        }
    }
    if (handle == null) return -10; // ECHILD
    const waited = win.WaitForSingleObject(handle, if (blocking) win.INFINITE else 0);
    if (!blocking and waited == win.WAIT_TIMEOUT) return -1;
    var exit_code: u32 = 0;
    if (win.GetExitCodeProcess(handle, &exit_code) == 0) return win_errno(win.GetLastError());
    var killed_sig: i64 = -1;
    {
        lock_procs();
        defer unlock_procs();
        for (&proc_slots) |*slot| {
            if (slot.used and slot.pid == pid) {
                killed_sig = slot.killed_sig;
                slot.used = false;
                slot.handle = null;
                break;
            }
        }
    }
    _ = win.CloseHandle(handle);
    if (killed_sig > 0) return 256 + killed_sig;
    return @intCast(exit_code);
}

/// Waits for a process to end. Gives its exit code, or 256 plus the signal that ended it.
export fn process_wait(pid: i64) i64 {
    if (is_windows) return wait_win(pid, true);
    var status: u32 = 0;
    while (true) {
        const rc = result(linux.syscall4(.wait4, fd_of(pid), @intFromPtr(&status), 0, 0));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        return decode(status);
    }
}

fn decode(status: u32) i64 {
    if (status & 0x7f == 0) return (status >> 8) & 0xff;
    return 256 + @as(i64, status & 0x7f);
}

/// Like `process_wait` without waiting: -1 while the process runs.
export fn process_try_wait(pid: i64) i64 {
    if (is_windows) return wait_win(pid, false);
    var status: u32 = 0;
    const wnohang: usize = 1;
    while (true) {
        const rc = result(linux.syscall4(.wait4, fd_of(pid), @intFromPtr(&status), wnohang, 0));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        if (rc == 0) return -1;
        return decode(status);
    }
}

export fn process_kill(pid: i64, number: i64) i64 {
    if (is_windows) {
        var handle: win.HANDLE = null;
        {
            lock_procs();
            defer unlock_procs();
            for (&proc_slots) |*slot| {
                if (slot.used and slot.pid == pid) {
                    slot.killed_sig = number;
                    handle = slot.handle;
                    break;
                }
            }
        }
        if (handle == null) return -3; // ESRCH
        if (win.TerminateProcess(handle, @intCast(256 + @max(number, 0))) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall2(.kill, fd_of(pid), @intCast(number)));
    }
}

/// Reads up to `count` bytes into the buffer; 0 at the end.
export fn process_read(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const into = buffer[0..@intCast(@max(@min(count, length), 0))];
    if (is_windows) {
        if (fd < 0) return -9;
        const want: u32 = @intCast(@min(into.len, std.math.maxInt(u32)));
        var got: u32 = 0;
        const h: win.HANDLE = @ptrFromInt(@as(usize, @intCast(fd)));
        if (win.ReadFile(h, into.ptr, want, &got, null) == 0) {
            const err = win.GetLastError();
            if (err == 109) return 0; // ERROR_BROKEN_PIPE -> EOF
            return win_errno(err);
        }
        return @intCast(got);
    } else {
        while (true) {
            const rc = result(linux.syscall3(.read, fd_of(fd), @intFromPtr(into.ptr), into.len));
            if (rc != eintr) return rc;
        }
    }
}

/// Writes every byte of the buffer, or gives the negated errno (a closed pipe is -32).
export fn process_write(fd: i64, buffer: [*]const u8, length: i64, count: i64) i64 {
    const bytes = buffer[0..@intCast(@max(@min(count, length), 0))];
    if (is_windows) {
        if (fd < 0) return -9;
        const h: win.HANDLE = @ptrFromInt(@as(usize, @intCast(fd)));
        var done: usize = 0;
        while (done < bytes.len) {
            const chunk: u32 = @intCast(@min(bytes.len - done, std.math.maxInt(u32)));
            var wrote: u32 = 0;
            if (win.WriteFile(h, bytes.ptr + done, chunk, &wrote, null) == 0) return win_errno(win.GetLastError());
            done += wrote;
        }
        return 0;
    } else {
        var done: usize = 0;
        while (done < bytes.len) {
            const rc = result(linux.syscall3(.write, fd_of(fd), @intFromPtr(bytes.ptr + done), bytes.len - done));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            done += @intCast(rc);
        }
        return 0;
    }
}

export fn process_close(fd: i64) i64 {
    if (is_windows) {
        if (fd < 0) return -9;
        if (win.CloseHandle(@ptrFromInt(@as(usize, @intCast(fd)))) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall1(.close, fd_of(fd)));
    }
}
