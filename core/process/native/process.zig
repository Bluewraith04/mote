//! The process natives of the `process` library: each gives a result or the negated errno.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
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

const eintr: i64 = -@as(i64, @intFromEnum(linux.E.INTR));
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

/// Starts a program. `modes` holds the stream of each of standard input, output and error in two bits (0 inherited, 1 a pipe, 2 the null device, 3 for error only: standard output). `fds` receives three 32-bit little-endian descriptors, the parent's end of each pipe or -1. `env_inherit` is 1 to pass this process's environment, else the NUL-ended strings of `envp`. Returns the process id.
export fn process_spawn(program: [*:0]const u8, argv: [*]const u8, argv_length: i64, envp: [*]const u8, envp_length: i64, env_inherit: i64, cwd: [*:0]const u8, modes: i64, fds: [*]u8, fds_length: i64) i64 {
    _ = fds_length;
    const arg_block = argv[0..@intCast(argv_length)];
    const env_block = envp[0..@intCast(envp_length)];
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

/// Waits for a process to end. Gives its exit code, or 256 plus the signal that ended it.
export fn process_wait(pid: i64) i64 {
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
    return result(linux.syscall2(.kill, fd_of(pid), @intCast(number)));
}

/// Reads up to `count` bytes into the buffer; 0 at the end.
export fn process_read(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const into = buffer[0..@intCast(@max(@min(count, length), 0))];
    while (true) {
        const rc = result(linux.syscall3(.read, fd_of(fd), @intFromPtr(into.ptr), into.len));
        if (rc != eintr) return rc;
    }
}

/// Writes every byte of the buffer, or gives the negated errno (a closed pipe is -32).
export fn process_write(fd: i64, buffer: [*]const u8, length: i64, count: i64) i64 {
    const bytes = buffer[0..@intCast(@max(@min(count, length), 0))];
    var done: usize = 0;
    while (done < bytes.len) {
        const rc = result(linux.syscall3(.write, fd_of(fd), @intFromPtr(bytes.ptr + done), bytes.len - done));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        done += @intCast(rc);
    }
    return 0;
}

export fn process_close(fd: i64) i64 {
    return result(linux.syscall1(.close, fd_of(fd)));
}
