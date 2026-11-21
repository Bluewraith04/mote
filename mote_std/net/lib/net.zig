//! The socket functions of the `net` library: each gives a count, a descriptor or the negated errno.
//! A name that does not resolve is -10000 minus the resolver's code; a wait that runs out is -4097.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const AddrInfo = if (is_windows) extern struct {
    flags: c_int,
    family: c_int,
    socktype: c_int,
    protocol: c_int,
    addrlen: usize,
    canonname: ?[*:0]u8,
    addr: ?*anyopaque,
    next: ?*AddrInfo,
} else extern struct {
    flags: c_int,
    family: c_int,
    socktype: c_int,
    protocol: c_int,
    addrlen: u32,
    addr: ?*anyopaque,
    canonname: ?[*:0]u8,
    next: ?*AddrInfo,
};

const getaddrinfo = if (is_windows)
    struct {
        extern "ws2_32" fn getaddrinfo(node: ?[*:0]const u8, service: ?[*:0]const u8, hints: *const AddrInfo, res: *?*AddrInfo) callconv(.winapi) c_int;
    }.getaddrinfo
else
    struct {
        extern fn getaddrinfo(node: ?[*:0]const u8, service: ?[*:0]const u8, hints: *const AddrInfo, res: *?*AddrInfo) c_int;
    }.getaddrinfo;

const freeaddrinfo = if (is_windows)
    struct {
        extern "ws2_32" fn freeaddrinfo(res: *AddrInfo) callconv(.winapi) void;
    }.freeaddrinfo
else
    struct {
        extern fn freeaddrinfo(res: *AddrInfo) void;
    }.freeaddrinfo;

const inet_ntop = if (is_windows)
    struct {
        extern "ws2_32" fn inet_ntop(family: c_int, source: *const anyopaque, destination: [*]u8, size: c_uint) callconv(.winapi) ?[*:0]const u8;
    }.inet_ntop
else
    struct {
        extern fn inet_ntop(family: c_int, source: *const anyopaque, destination: [*]u8, size: c_uint) ?[*:0]const u8;
    }.inet_ntop;

const wsa = struct {
    const SOCKET = usize;
    const INVALID_SOCKET: SOCKET = std.math.maxInt(usize);
    const POLLRDNORM: i16 = 0x0100;
    const POLLWRNORM: i16 = 0x0010;
    const POLLERR: i16 = 0x0001;
    const POLLHUP: i16 = 0x0002;
    const FIONBIO: u32 = 0x8004667E;
    const SOL_SOCKET: c_int = 0xFFFF;
    const SO_EXCLUSIVEADDRUSE: c_int = -5;
    const SO_ERROR: c_int = 0x1007;

    const WSAPollFd = extern struct {
        fd: SOCKET,
        events: i16,
        revents: i16,
    };

    const WSAData = extern struct {
        bytes: [512]u8,
    };

    extern "ws2_32" fn WSAStartup(wVersionRequested: u16, lpWSAData: *WSAData) callconv(.winapi) c_int;
    extern "ws2_32" fn WSAGetLastError() callconv(.winapi) c_int;
    extern "ws2_32" fn WSAPoll(fdArray: [*]WSAPollFd, fds: u32, timeout: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn socket(af: c_int, kind: c_int, protocol: c_int) callconv(.winapi) SOCKET;
    extern "ws2_32" fn connect(s: SOCKET, name: *const anyopaque, namelen: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn bind(s: SOCKET, name: *const anyopaque, namelen: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn listen(s: SOCKET, backlog: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn accept(s: SOCKET, addr: ?*anyopaque, addrlen: ?*c_int) callconv(.winapi) SOCKET;
    extern "ws2_32" fn recvfrom(s: SOCKET, buf: [*]u8, len: c_int, flags: c_int, from: ?*anyopaque, fromlen: ?*c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn sendto(s: SOCKET, buf: [*]const u8, len: c_int, flags: c_int, to: ?*const anyopaque, tolen: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn shutdown(s: SOCKET, how: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn closesocket(s: SOCKET) callconv(.winapi) c_int;
    extern "ws2_32" fn ioctlsocket(s: SOCKET, cmd: u32, argp: *u32) callconv(.winapi) c_int;
    extern "ws2_32" fn getsockopt(s: SOCKET, level: c_int, optname: c_int, optval: *c_int, optlen: *c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn setsockopt(s: SOCKET, level: c_int, optname: c_int, optval: *const c_int, optlen: c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn getsockname(s: SOCKET, name: *anyopaque, namelen: *c_int) callconv(.winapi) c_int;
    extern "ws2_32" fn getpeername(s: SOCKET, name: *anyopaque, namelen: *c_int) callconv(.winapi) c_int;
};

var wsa_ready: std.atomic.Value(bool) = .init(false);

fn ensure_wsa() void {
    if (!is_windows) return;
    if (wsa_ready.load(.acquire)) return;
    var data: wsa.WSAData = undefined;
    _ = wsa.WSAStartup(0x0202, &data);
    wsa_ready.store(true, .release);
}

fn wsa_errno(code: c_int) i64 {
    return switch (code) {
        10004 => -4, // EINTR
        10009, 10038 => -9, // EBADF / ENOTSOCK
        10013 => -13, // EACCES
        10022 => -22, // EINVAL
        10035 => -11, // EWOULDBLOCK
        10047 => -97, // EAFNOSUPPORT
        10048 => -98, // EADDRINUSE
        10049 => -99, // EADDRNOTAVAIL
        10051 => -101, // ENETUNREACH
        10053, 10054 => -104, // ECONNRESET
        10057 => -107, // ENOTCONN
        10058 => -32, // EPIPE
        10060 => -110, // ETIMEDOUT
        10061 => -111, // ECONNREFUSED
        10065 => -113, // EHOSTUNREACH
        else => -5,
    };
}

const eintr: i64 = -4;
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

fn sock_of(fd: i64) wsa.SOCKET {
    if (fd < 0) return wsa.INVALID_SOCKET;
    return @intCast(fd);
}

/// 1 when the descriptor is ready, 0 when the time ran out, else the negated errno.
fn wait(fd: i64, events: i16, timeout_ms: i64) i64 {
    if (is_windows) {
        if (fd < 0) return -9;
        const limit: c_int = if (timeout_ms < 0) -1 else @intCast(@min(timeout_ms, 2_000_000_000));
        const ev: i16 = if (events == pollout) wsa.POLLWRNORM else wsa.POLLRDNORM;
        var entry = wsa.WSAPollFd{ .fd = sock_of(fd), .events = ev, .revents = 0 };
        const rc = wsa.WSAPoll((&entry)[0..1].ptr, 1, limit);
        if (rc < 0) return wsa_errno(wsa.WSAGetLastError());
        return rc;
    } else {
        var entry = linux.pollfd{ .fd = @intCast(fd), .events = events, .revents = 0 };
        const limit: isize = if (timeout_ms < 0) -1 else @intCast(@min(timeout_ms, 2_000_000_000));
        while (true) {
            const rc = result(linux.syscall3(.poll, @intFromPtr(&entry), 1, @bitCast(limit)));
            if (rc == eintr) continue;
            return rc;
        }
    }
}

/// The addresses a name and a port stand for, to free with `freeaddrinfo`; or a negative code.
fn resolve(host: [*:0]const u8, port: i64, socktype: c_int, passive: bool, found: *?*AddrInfo) i64 {
    ensure_wsa();
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
    if (is_windows) {
        var mode: u32 = 0;
        _ = wsa.ioctlsocket(sock_of(fd), wsa.FIONBIO, &mode);
    } else {
        const flags = linux.syscall3(.fcntl, fd_of(fd), 3, 0);
        _ = linux.syscall3(.fcntl, fd_of(fd), 4, flags & ~@as(usize, sock_nonblock));
    }
}

/// Connects a stream socket to the first address of the name that answers within the time.
export fn net_connect(host: [*:0]const u8, port: i64, timeout_ms: i64) i64 {
    ensure_wsa();
    var found: ?*AddrInfo = null;
    const code = resolve(host, port, sock_stream, false, &found);
    if (code < 0) return code;
    defer freeaddrinfo(found.?);
    var last: i64 = -111; // ECONNREFUSED
    var at = found;
    while (at) |entry| : (at = entry.next) {
        if (is_windows) {
            const s = wsa.socket(entry.family, entry.socktype, entry.protocol);
            if (s == wsa.INVALID_SOCKET) {
                last = wsa_errno(wsa.WSAGetLastError());
                continue;
            }
            const fd: i64 = @intCast(s);
            var nb: u32 = 1;
            _ = wsa.ioctlsocket(s, wsa.FIONBIO, &nb);
            const rc: i64 = if (wsa.connect(s, entry.addr.?, @intCast(entry.addrlen)) == 0) 0 else blk: {
                const err = wsa.WSAGetLastError();
                if (err == 10035 or err == 10036) {
                    const ready = wait(fd, pollout, timeout_ms);
                    if (ready == 0) {
                        _ = wsa.closesocket(s);
                        last = timed_out;
                        continue;
                    }
                    if (ready < 0) break :blk ready;
                    var failure: c_int = 0;
                    var len: c_int = 4;
                    if (wsa.getsockopt(s, wsa.SOL_SOCKET, wsa.SO_ERROR, &failure, &len) != 0) {
                        break :blk wsa_errno(wsa.WSAGetLastError());
                    }
                    if (failure != 0) break :blk wsa_errno(failure);
                    break :blk 0;
                }
                break :blk wsa_errno(err);
            };
            if (rc < 0) {
                _ = wsa.closesocket(s);
                last = rc;
                continue;
            }
            set_blocking(fd);
            const one: c_int = 1;
            _ = wsa.setsockopt(s, ipproto_tcp, tcp_nodelay, &one, 4);
            return fd;
        } else {
            const fd = result(linux.syscall3(.socket, @intCast(entry.family), @intCast(entry.socktype | sock_nonblock | sock_cloexec), @intCast(entry.protocol)));
            if (fd < 0) {
                last = fd;
                continue;
            }
            var rc = result(linux.syscall3(.connect, fd_of(fd), @intFromPtr(entry.addr), @intCast(entry.addrlen)));
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
    }
    return last;
}

/// Binds a socket of the kind (1 stream, 2 datagram) to the address and, for a stream, listens.
fn bind_to(host: [*:0]const u8, port: i64, socktype: c_int, backlog: i64) i64 {
    ensure_wsa();
    var found: ?*AddrInfo = null;
    const code = resolve(host, port, socktype, true, &found);
    if (code < 0) return code;
    defer freeaddrinfo(found.?);
    var last: i64 = -99; // EADDRNOTAVAIL
    var at = found;
    while (at) |entry| : (at = entry.next) {
        if (is_windows) {
            const s = wsa.socket(entry.family, entry.socktype, entry.protocol);
            if (s == wsa.INVALID_SOCKET) {
                last = wsa_errno(wsa.WSAGetLastError());
                continue;
            }
            const one: c_int = 1;
            _ = wsa.setsockopt(s, wsa.SOL_SOCKET, wsa.SO_EXCLUSIVEADDRUSE, &one, 4);
            var ok = wsa.bind(s, entry.addr.?, @intCast(entry.addrlen)) == 0;
            if (ok and socktype == sock_stream) ok = wsa.listen(s, @intCast(backlog)) == 0;
            if (!ok) {
                last = wsa_errno(wsa.WSAGetLastError());
                _ = wsa.closesocket(s);
                continue;
            }
            return @intCast(s);
        } else {
            const fd = result(linux.syscall3(.socket, @intCast(entry.family), @intCast(entry.socktype | sock_cloexec), @intCast(entry.protocol)));
            if (fd < 0) {
                last = fd;
                continue;
            }
            const one: c_int = 1;
            _ = linux.syscall5(.setsockopt, fd_of(fd), sol_socket, so_reuseaddr, @intFromPtr(&one), 4);
            var rc = result(linux.syscall3(.bind, fd_of(fd), @intFromPtr(entry.addr), @intCast(entry.addrlen)));
            if (rc == 0 and socktype == sock_stream) rc = result(linux.syscall2(.listen, fd_of(fd), @intCast(backlog)));
            if (rc < 0) {
                _ = linux.syscall1(.close, fd_of(fd));
                last = rc;
                continue;
            }
            return fd;
        }
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
    if (is_windows) {
        const s = wsa.accept(sock_of(fd), null, null);
        if (s == wsa.INVALID_SOCKET) return wsa_errno(wsa.WSAGetLastError());
        const one: c_int = 1;
        _ = wsa.setsockopt(s, ipproto_tcp, tcp_nodelay, &one, 4);
        return @intCast(s);
    } else {
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
}

/// Reads up to the buffer's length once bytes are there: the count, 0 at the end of the stream, or -4097 when the time ran out.
export fn net_read(fd: i64, buffer: [*]u8, length: i64, timeout_ms: i64) i64 {
    const ready = wait(fd, pollin, timeout_ms);
    if (ready < 0) return ready;
    if (ready == 0) return timed_out;
    if (is_windows) {
        const want: c_int = @intCast(@min(@max(length, 0), std.math.maxInt(c_int)));
        const rc = wsa.recvfrom(sock_of(fd), buffer, want, 0, null, null);
        if (rc < 0) return wsa_errno(wsa.WSAGetLastError());
        return rc;
    } else {
        while (true) {
            const rc = result(linux.syscall6(.recvfrom, fd_of(fd), @intFromPtr(buffer), @intCast(@max(length, 0)), 0, 0, 0));
            if (rc == eintr) continue;
            return rc;
        }
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
        if (is_windows) {
            const chunk: c_int = @intCast(@min(total - done, std.math.maxInt(c_int)));
            const rc = wsa.sendto(sock_of(fd), buffer + done, chunk, 0, null, 0);
            if (rc < 0) return wsa_errno(wsa.WSAGetLastError());
            done += @intCast(rc);
        } else {
            const rc = result(linux.syscall6(.sendto, fd_of(fd), @intFromPtr(buffer) + done, total - done, msg_nosignal, 0, 0));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            done += @intCast(rc);
        }
    }
    return @intCast(done);
}

export fn net_shutdown(fd: i64, how: i64) i64 {
    if (is_windows) {
        if (wsa.shutdown(sock_of(fd), @intCast(how)) != 0) return wsa_errno(wsa.WSAGetLastError());
        return 0;
    } else {
        return result(linux.syscall2(.shutdown, fd_of(fd), @intCast(how)));
    }
}

export fn net_close(fd: i64) i64 {
    if (is_windows) {
        if (wsa.closesocket(sock_of(fd)) != 0) return wsa_errno(wsa.WSAGetLastError());
        return 0;
    } else {
        return result(linux.syscall1(.close, fd_of(fd)));
    }
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
    } else if (is_windows) {
        var wsize: c_int = @intCast(size);
        const rc = if (peer != 0) wsa.getpeername(sock_of(fd), &storage, &wsize) else wsa.getsockname(sock_of(fd), &storage, &wsize);
        if (rc != 0) return wsa_errno(wsa.WSAGetLastError());
    } else {
        const rc = result(linux.syscall3(if (peer != 0) .getpeername else .getsockname, fd_of(fd), @intFromPtr(&storage), @intFromPtr(&size)));
        if (rc < 0) return rc;
    }
    var text: [64]u8 = undefined;
    var used: usize = 0;
    var port: u16 = 0;
    const af_inet6: u16 = if (is_windows) 23 else 10;
    if (storage.family == 2) {
        port = std.mem.readInt(u16, storage.data[0..2], .big);
        const name = inet_ntop(2, &storage.data[2], &text, text.len) orelse return -22;
        used = std.mem.len(name);
    } else if (storage.family == af_inet6) {
        port = std.mem.readInt(u16, storage.data[0..2], .big);
        text[0] = '[';
        const name = inet_ntop(af_inet6, &storage.data[6], text[1..].ptr, text.len - 1) orelse return -22;
        used = 1 + std.mem.len(name);
        text[used] = ']';
        used += 1;
    } else {
        return -97; // EAFNOSUPPORT
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
    if (is_windows) {
        var wsize: c_int = @sizeOf(Storage);
        if (wsa.getsockname(sock_of(fd), &storage, &wsize) != 0) return wsa_errno(wsa.WSAGetLastError());
    } else {
        var size: u32 = @sizeOf(Storage);
        const rc = result(linux.syscall3(.getsockname, fd_of(fd), @intFromPtr(&storage), @intFromPtr(&size)));
        if (rc < 0) return rc;
    }
    return std.mem.readInt(u16, storage.data[0..2], .big);
}

/// Sends a datagram to a name and a port.
export fn net_udp_send(fd: i64, host: [*:0]const u8, port: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    var found: ?*AddrInfo = null;
    const code = resolve(host, port, sock_dgram, false, &found);
    if (code < 0) return code;
    defer freeaddrinfo(found.?);
    const total: usize = @intCast(@max(@min(count, length), 0));
    var last: i64 = -99; // EADDRNOTAVAIL
    var at = found;
    while (at) |entry| : (at = entry.next) {
        if (is_windows) {
            const chunk: c_int = @intCast(@min(total, std.math.maxInt(c_int)));
            const rc = wsa.sendto(sock_of(fd), buffer, chunk, 0, entry.addr, @intCast(entry.addrlen));
            if (rc >= 0) return rc;
            last = wsa_errno(wsa.WSAGetLastError());
        } else {
            const rc = result(linux.syscall6(.sendto, fd_of(fd), @intFromPtr(buffer), total, msg_nosignal, @intFromPtr(entry.addr), @intCast(entry.addrlen)));
            if (rc >= 0) return rc;
            last = rc;
        }
    }
    return last;
}

/// Receives one datagram: the count, with the sender as `address:port` in `peer` and its length in `sender[0]`; -4097 when the time ran out.
export fn net_udp_recv(fd: i64, buffer: [*]u8, length: i64, timeout_ms: i64, peer: [*]u8, peer_length: i64, sender: [*]i64, sender_length: i64) i64 {
    if (sender_length < 1) return -22;
    const ready = wait(fd, pollin, timeout_ms);
    if (ready < 0) return ready;
    if (ready == 0) return timed_out;
    var storage: Storage align(8) = undefined;
    if (is_windows) {
        var wsize: c_int = @sizeOf(Storage);
        const want: c_int = @intCast(@min(@max(length, 0), std.math.maxInt(c_int)));
        const rc = wsa.recvfrom(sock_of(fd), buffer, want, 0, &storage, &wsize);
        if (rc < 0) return wsa_errno(wsa.WSAGetLastError());
        sender[0] = text_of(fd, 1, peer, peer_length, &storage);
        return rc;
    } else {
        var size: u32 = @sizeOf(Storage);
        while (true) {
            const rc = result(linux.syscall6(.recvfrom, fd_of(fd), @intFromPtr(buffer), @intCast(@max(length, 0)), 0, @intFromPtr(&storage), @intFromPtr(&size)));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            sender[0] = text_of(fd, 1, peer, peer_length, &storage);
            return rc;
        }
    }
}
