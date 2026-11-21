//! The socket natives of the `net` library: each gives a count, a descriptor or the negated errno.
//! A name that does not resolve is -10000 minus the resolver's code; a wait that runs out is -4097.

const std = @import("std");
const linux = std.os.linux;

/// A fault in a native stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const AddrInfo = extern struct {
    flags: c_int,
    family: c_int,
    socktype: c_int,
    protocol: c_int,
    addrlen: u32,
    addr: ?*anyopaque,
    canonname: ?[*:0]u8,
    next: ?*AddrInfo,
};

extern fn getaddrinfo(node: ?[*:0]const u8, service: ?[*:0]const u8, hints: *const AddrInfo, res: *?*AddrInfo) c_int;
extern fn freeaddrinfo(res: *AddrInfo) void;
extern fn inet_ntop(family: c_int, source: *const anyopaque, destination: [*]u8, size: c_uint) ?[*:0]const u8;

const eintr: i64 = -@as(i64, @intFromEnum(linux.E.INTR));
const timed_out: i64 = -4097;
const sock_stream = 1;
const sock_dgram = 2;
const sock_nonblock = 0o4000;
const sock_cloexec = 0o2000000;
const sol_socket = 1;
const so_reuseaddr = 2;
const so_error = 4;
const ipproto_tcp = 6;
const tcp_nodelay = 1;
const msg_nosignal = 0x4000;
const pollin: i16 = 1;
const pollout: i16 = 4;
const ai_passive = 1;

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn fd_of(fd: i64) usize {
    return @bitCast(fd);
}

/// 1 when the descriptor is ready, 0 when the time ran out, else the negated errno.
fn wait(fd: i64, events: i16, timeout_ms: i64) i64 {
    var entry = linux.pollfd{ .fd = @intCast(fd), .events = events, .revents = 0 };
    const limit: isize = if (timeout_ms < 0) -1 else @intCast(@min(timeout_ms, 2_000_000_000));
    while (true) {
        const rc = result(linux.syscall3(.poll, @intFromPtr(&entry), 1, @bitCast(limit)));
        if (rc == eintr) continue;
        return rc;
    }
}

/// The addresses a name and a port stand for, to free with `freeaddrinfo`; or a negative code.
fn resolve(host: [*:0]const u8, port: i64, socktype: c_int, passive: bool, found: *?*AddrInfo) i64 {
    var service: [8:0]u8 = undefined;
    var digits: [5]u8 = undefined;
    var count: usize = 0;
    var rest: u32 = @intCast(@max(@min(port, 65535), 0));
    while (true) {
        digits[count] = '0' + @as(u8, @intCast(rest % 10));
        count += 1;
        rest /= 10;
        if (rest == 0) break;
    }
    for (0..count) |at| service[at] = digits[count - 1 - at];
    service[count] = 0;
    var hints = std.mem.zeroes(AddrInfo);
    hints.socktype = socktype;
    if (passive) hints.flags = ai_passive;
    const name: ?[*:0]const u8 = if (host[0] == 0) null else host;
    const code = getaddrinfo(name, &service, &hints, found);
    if (code != 0) return -10000 - @as(i64, if (code < 0) -code else code);
    return 0;
}

fn set_blocking(fd: i64) void {
    const flags = linux.syscall3(.fcntl, fd_of(fd), 3, 0);
    _ = linux.syscall3(.fcntl, fd_of(fd), 4, flags & ~@as(usize, sock_nonblock));
}

/// Connects a stream socket to the first address of the name that answers within the time.
export fn net_connect(host: [*:0]const u8, port: i64, timeout_ms: i64) i64 {
    var found: ?*AddrInfo = null;
    const code = resolve(host, port, sock_stream, false, &found);
    if (code < 0) return code;
    defer freeaddrinfo(found.?);
    var last: i64 = -@as(i64, @intFromEnum(linux.E.CONNREFUSED));
    var at = found;
    while (at) |entry| : (at = entry.next) {
        const fd = result(linux.syscall3(.socket, @intCast(entry.family), @intCast(entry.socktype | sock_nonblock | sock_cloexec), @intCast(entry.protocol)));
        if (fd < 0) {
            last = fd;
            continue;
        }
        var rc = result(linux.syscall3(.connect, fd_of(fd), @intFromPtr(entry.addr), entry.addrlen));
        if (rc == -@as(i64, @intFromEnum(linux.E.INPROGRESS))) {
            const ready = wait(fd, pollout, timeout_ms);
            if (ready == 0) {
                _ = linux.syscall1(.close, fd_of(fd));
                last = timed_out;
                continue;
            }
            var failure: c_int = 0;
            var length: u32 = 4;
            rc = if (ready < 0) ready else result(linux.syscall5(.getsockopt, fd_of(fd), sol_socket, so_error, @intFromPtr(&failure), @intFromPtr(&length)));
            if (rc == 0 and failure != 0) rc = -@as(i64, failure);
        }
        if (rc < 0) {
            _ = linux.syscall1(.close, fd_of(fd));
            last = rc;
            continue;
        }
        set_blocking(fd);
        const one: c_int = 1;
        _ = linux.syscall5(.setsockopt, fd_of(fd), ipproto_tcp, tcp_nodelay, @intFromPtr(&one), 4);
        return fd;
    }
    return last;
}

/// Binds a socket of the kind (1 stream, 2 datagram) to the address and, for a stream, listens.
fn bind_to(host: [*:0]const u8, port: i64, socktype: c_int, backlog: i64) i64 {
    var found: ?*AddrInfo = null;
    const code = resolve(host, port, socktype, true, &found);
    if (code < 0) return code;
    defer freeaddrinfo(found.?);
    var last: i64 = -@as(i64, @intFromEnum(linux.E.ADDRNOTAVAIL));
    var at = found;
    while (at) |entry| : (at = entry.next) {
        const fd = result(linux.syscall3(.socket, @intCast(entry.family), @intCast(entry.socktype | sock_cloexec), @intCast(entry.protocol)));
        if (fd < 0) {
            last = fd;
            continue;
        }
        const one: c_int = 1;
        _ = linux.syscall5(.setsockopt, fd_of(fd), sol_socket, so_reuseaddr, @intFromPtr(&one), 4);
        var rc = result(linux.syscall3(.bind, fd_of(fd), @intFromPtr(entry.addr), entry.addrlen));
        if (rc == 0 and socktype == sock_stream) rc = result(linux.syscall2(.listen, fd_of(fd), @intCast(backlog)));
        if (rc < 0) {
            _ = linux.syscall1(.close, fd_of(fd));
            last = rc;
            continue;
        }
        return fd;
    }
    return last;
}

export fn net_listen(host: [*:0]const u8, port: i64, backlog: i64) i64 {
    return bind_to(host, port, sock_stream, backlog);
}

export fn net_udp_bind(host: [*:0]const u8, port: i64) i64 {
    return bind_to(host, port, sock_dgram, 0);
}

/// Waits up to the time for a connection and gives its descriptor.
export fn net_accept(fd: i64, timeout_ms: i64) i64 {
    const ready = wait(fd, pollin, timeout_ms);
    if (ready < 0) return ready;
    if (ready == 0) return timed_out;
    while (true) {
        const rc = result(linux.syscall4(.accept4, fd_of(fd), 0, 0, sock_cloexec));
        if (rc == eintr) continue;
        if (rc >= 0) {
            const one: c_int = 1;
            _ = linux.syscall5(.setsockopt, fd_of(rc), ipproto_tcp, tcp_nodelay, @intFromPtr(&one), 4);
        }
        return rc;
    }
}

/// Reads up to the buffer's length once bytes are there: the count, 0 at the end of the stream, or -4097 when the time ran out.
export fn net_read(fd: i64, buffer: [*]u8, length: i64, timeout_ms: i64) i64 {
    const ready = wait(fd, pollin, timeout_ms);
    if (ready < 0) return ready;
    if (ready == 0) return timed_out;
    while (true) {
        const rc = result(linux.syscall6(.recvfrom, fd_of(fd), @intFromPtr(buffer), @intCast(@max(length, 0)), 0, 0, 0));
        if (rc == eintr) continue;
        return rc;
    }
}

/// Writes the first `count` bytes, waiting up to the time for room at each step.
export fn net_write(fd: i64, buffer: [*]u8, length: i64, count: i64, timeout_ms: i64) i64 {
    const total: usize = @intCast(@max(@min(count, length), 0));
    var done: usize = 0;
    while (done < total) {
        const ready = wait(fd, pollout, timeout_ms);
        if (ready < 0) return ready;
        if (ready == 0) return timed_out;
        const rc = result(linux.syscall6(.sendto, fd_of(fd), @intFromPtr(buffer) + done, total - done, msg_nosignal, 0, 0));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        done += @intCast(rc);
    }
    return @intCast(done);
}

export fn net_shutdown(fd: i64, how: i64) i64 {
    return result(linux.syscall2(.shutdown, fd_of(fd), @intCast(how)));
}

export fn net_close(fd: i64) i64 {
    return result(linux.syscall1(.close, fd_of(fd)));
}

const Storage = extern struct {
    family: u16,
    data: [126]u8,
};

/// Writes `address:port` of the local (`peer` 0) or remote end into the buffer and gives its length; an IPv6 address is in brackets.
export fn net_address(fd: i64, peer: i64, buffer: [*]u8, length: i64) i64 {
    return text_of(fd, peer, buffer, length, null);
}

fn text_of(fd: i64, peer: i64, buffer: [*]u8, length: i64, from: ?*const Storage) i64 {
    var storage: Storage align(8) = undefined;
    var size: u32 = @sizeOf(Storage);
    if (from) |known| {
        storage = known.*;
    } else {
        const rc = result(linux.syscall3(if (peer != 0) .getpeername else .getsockname, fd_of(fd), @intFromPtr(&storage), @intFromPtr(&size)));
        if (rc < 0) return rc;
    }
    var text: [64]u8 = undefined;
    var used: usize = 0;
    var port: u16 = 0;
    if (storage.family == 2) {
        port = std.mem.readInt(u16, storage.data[0..2], .big);
        const name = inet_ntop(2, &storage.data[2], &text, text.len) orelse return -@as(i64, @intFromEnum(linux.E.INVAL));
        used = std.mem.len(name);
    } else if (storage.family == 10) {
        port = std.mem.readInt(u16, storage.data[0..2], .big);
        text[0] = '[';
        const name = inet_ntop(10, &storage.data[6], text[1..].ptr, text.len - 1) orelse return -@as(i64, @intFromEnum(linux.E.INVAL));
        used = 1 + std.mem.len(name);
        text[used] = ']';
        used += 1;
    } else {
        return -@as(i64, @intFromEnum(linux.E.AFNOSUPPORT));
    }
    var digits: [5]u8 = undefined;
    var count: usize = 0;
    var rest: u32 = port;
    while (true) {
        digits[count] = '0' + @as(u8, @intCast(rest % 10));
        count += 1;
        rest /= 10;
        if (rest == 0) break;
    }
    text[used] = ':';
    used += 1;
    for (0..count) |at| text[used + at] = digits[count - 1 - at];
    used += count;
    const out = buffer[0..@intCast(@max(length, 0))];
    @memcpy(out[0..@min(out.len, used)], text[0..@min(out.len, used)]);
    return @intCast(used);
}

/// The port the socket is bound to.
export fn net_port(fd: i64) i64 {
    var storage: Storage align(8) = undefined;
    var size: u32 = @sizeOf(Storage);
    const rc = result(linux.syscall3(.getsockname, fd_of(fd), @intFromPtr(&storage), @intFromPtr(&size)));
    if (rc < 0) return rc;
    return std.mem.readInt(u16, storage.data[0..2], .big);
}

/// Sends a datagram to a name and a port.
export fn net_udp_send(fd: i64, host: [*:0]const u8, port: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    var found: ?*AddrInfo = null;
    const code = resolve(host, port, sock_dgram, false, &found);
    if (code < 0) return code;
    defer freeaddrinfo(found.?);
    const total: usize = @intCast(@max(@min(count, length), 0));
    var last: i64 = -@as(i64, @intFromEnum(linux.E.ADDRNOTAVAIL));
    var at = found;
    while (at) |entry| : (at = entry.next) {
        const rc = result(linux.syscall6(.sendto, fd_of(fd), @intFromPtr(buffer), total, msg_nosignal, @intFromPtr(entry.addr), entry.addrlen));
        if (rc >= 0) return rc;
        last = rc;
    }
    return last;
}

/// Receives one datagram: the count, with the sender as `address:port` in `peer` and its length in `sender[0]`; -4097 when the time ran out.
export fn net_udp_recv(fd: i64, buffer: [*]u8, length: i64, timeout_ms: i64, peer: [*]u8, peer_length: i64, sender: [*]i64, sender_length: i64) i64 {
    if (sender_length < 1) return -@as(i64, @intFromEnum(linux.E.INVAL));
    const ready = wait(fd, pollin, timeout_ms);
    if (ready < 0) return ready;
    if (ready == 0) return timed_out;
    var storage: Storage align(8) = undefined;
    var size: u32 = @sizeOf(Storage);
    while (true) {
        const rc = result(linux.syscall6(.recvfrom, fd_of(fd), @intFromPtr(buffer), @intCast(@max(length, 0)), 0, @intFromPtr(&storage), @intFromPtr(&size)));
        if (rc == eintr) continue;
        if (rc < 0) return rc;
        sender[0] = text_of(fd, 1, peer, peer_length, &storage);
        return rc;
    }
}
