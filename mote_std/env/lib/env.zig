//! The environment functions of the `env` library: each gives a size or the negated errno.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

extern var environ: [*:null]?[*:0]u8;
extern fn setenv(name: [*:0]const u8, value: [*:0]const u8, overwrite: c_int) c_int;
extern fn unsetenv(name: [*:0]const u8) c_int;

const win = struct {
    extern "kernel32" fn GetLastError() callconv(.winapi) u32;
    extern "kernel32" fn GetEnvironmentVariableW(lpName: [*:0]const u16, lpBuffer: ?[*]u16, nSize: u32) callconv(.winapi) u32;
    extern "kernel32" fn SetEnvironmentVariableW(lpName: [*:0]const u16, lpValue: ?[*:0]const u16) callconv(.winapi) i32;
    extern "kernel32" fn GetEnvironmentStringsW() callconv(.winapi) ?[*:0]u16;
    extern "kernel32" fn FreeEnvironmentStringsW(penv: [*:0]u16) callconv(.winapi) i32;
    extern "kernel32" fn GetCurrentDirectoryW(nBufferLength: u32, lpBuffer: [*]u16) callconv(.winapi) u32;
    extern "kernel32" fn SetCurrentDirectoryW(lpPathName: [*:0]const u16) callconv(.winapi) i32;
    extern "kernel32" fn MultiByteToWideChar(CodePage: u32, dwFlags: u32, lpMultiByteStr: [*]const u8, cbMultiByte: i32, lpWideCharStr: ?[*]u16, cchWideChar: i32) callconv(.winapi) i32;
    extern "kernel32" fn WideCharToMultiByte(CodePage: u32, dwFlags: u32, lpWideCharStr: [*]const u16, cchWideChar: i32, lpMultiByteStr: ?[*]u8, cbMultiByte: i32, lpDefaultChar: ?[*:0]const u8, lpUsedDefaultChar: ?*i32) callconv(.winapi) i32;
};

fn win_to_utf16(alloc: std.mem.Allocator, text: [*:0]const u8) ?[:0]u16 {
    const slice = std.mem.span(text);
    if (slice.len == 0) {
        const empty = alloc.allocSentinel(u16, 0, 0) catch return null;
        return empty;
    }
    const needed = win.MultiByteToWideChar(65001, 0, slice.ptr, @intCast(slice.len), null, 0);
    if (needed <= 0) return null;
    const buf = alloc.allocSentinel(u16, @intCast(needed), 0) catch return null;
    const written = win.MultiByteToWideChar(65001, 0, slice.ptr, @intCast(slice.len), buf.ptr, needed);
    if (written <= 0) {
        alloc.free(buf);
        return null;
    }
    return buf;
}

fn win_to_utf8(alloc: std.mem.Allocator, wide: []const u16) ?[]u8 {
    if (wide.len == 0) return alloc.alloc(u8, 0) catch null;
    const needed = win.WideCharToMultiByte(65001, 0, wide.ptr, @intCast(wide.len), null, 0, null, null);
    if (needed <= 0) return null;
    const buf = alloc.alloc(u8, @intCast(needed)) catch return null;
    const written = win.WideCharToMultiByte(65001, 0, wide.ptr, @intCast(wide.len), buf.ptr, needed, null, null);
    if (written <= 0) {
        alloc.free(buf);
        return null;
    }
    return buf;
}

fn win_errno(err: u32) i64 {
    return switch (err) {
        2, 3, 161 => -2, // ENOENT
        5 => -13, // EACCES
        267 => -20, // ENOTDIR
        else => -22, // EINVAL
    };
}

fn result(rc: usize) i64 {
    return @as(isize, @bitCast(rc));
}

fn room(buffer: [*]u8, length: i64) []u8 {
    return buffer[0..@intCast(@max(length, 0))];
}

/// Copies the value of a variable into the buffer and gives its length, or -1 when it is not set; a length over the buffer's means it did not fit.
export fn env_get(name: [*:0]const u8, buffer: [*]u8, length: i64) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const wname = win_to_utf16(alloc, name) orelse return -1;
        defer alloc.free(wname);
        var cap: u32 = 512;
        while (true) {
            const wval = alloc.alloc(u16, cap) catch return -1;
            defer alloc.free(wval);
            const rc = win.GetEnvironmentVariableW(wname.ptr, wval.ptr, cap);
            if (rc == 0) {
                const err = win.GetLastError();
                if (err == 203) return -1; // ERROR_ENVVAR_NOT_FOUND
                return 0; // empty string variable
            }
            if (rc < cap) {
                const utf8 = win_to_utf8(alloc, wval[0..rc]) orelse return -1;
                defer alloc.free(utf8);
                const out = room(buffer, length);
                @memcpy(out[0..@min(out.len, utf8.len)], utf8[0..@min(out.len, utf8.len)]);
                return @intCast(utf8.len);
            }
            cap = rc;
        }
    } else {
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
}

export fn env_set(name: [*:0]const u8, value: [*:0]const u8) i64 {
    if (is_windows) {
        const name_slice = std.mem.span(name);
        if (name_slice.len == 0 or std.mem.indexOfScalar(u8, name_slice, '=') != null) return -22;
        const alloc = std.heap.page_allocator;
        const wname = win_to_utf16(alloc, name) orelse return -22;
        defer alloc.free(wname);
        const wval = win_to_utf16(alloc, value) orelse return -22;
        defer alloc.free(wval);
        if (win.SetEnvironmentVariableW(wname.ptr, wval.ptr) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        if (setenv(name, value, 1) != 0) return -@as(i64, @intFromEnum(linux.E.INVAL));
        return 0;
    }
}

export fn env_remove(name: [*:0]const u8) i64 {
    if (is_windows) {
        const name_slice = std.mem.span(name);
        if (name_slice.len == 0 or std.mem.indexOfScalar(u8, name_slice, '=') != null) return -22;
        const alloc = std.heap.page_allocator;
        const wname = win_to_utf16(alloc, name) orelse return -22;
        defer alloc.free(wname);
        if (win.SetEnvironmentVariableW(wname.ptr, null) == 0) {
            const err = win.GetLastError();
            if (err == 203) return 0; // ERROR_ENVVAR_NOT_FOUND
            return win_errno(err);
        }
        return 0;
    } else {
        if (unsetenv(name) != 0) return -@as(i64, @intFromEnum(linux.E.INVAL));
        return 0;
    }
}

/// Writes every `NAME=value` ended by a NUL into the buffer and gives the size they need.
export fn env_list(buffer: [*]u8, length: i64) i64 {
    const out = room(buffer, length);
    var needed: usize = 0;
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const raw = win.GetEnvironmentStringsW() orelse return 0;
        defer _ = win.FreeEnvironmentStringsW(raw);
        var ptr: [*]const u16 = raw;
        while (ptr[0] != 0) {
            var end: usize = 0;
            while (ptr[end] != 0) : (end += 1) {}
            const entry = ptr[0..end];
            ptr += end + 1;
            if (entry[0] == '=') continue;
            const utf8 = win_to_utf8(alloc, entry) orelse continue;
            defer alloc.free(utf8);
            if (needed + utf8.len + 1 <= out.len) {
                @memcpy(out[needed..][0..utf8.len], utf8);
                out[needed + utf8.len] = 0;
            }
            needed += utf8.len + 1;
        }
        return @intCast(needed);
    } else {
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
}

/// Writes the working directory into the buffer and gives its length, which is more than the buffer's when it did not fit.
export fn env_cwd(buffer: [*]u8, length: i64) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        var cap: u32 = 512;
        while (true) {
            const wbuf = alloc.alloc(u16, cap) catch return -12;
            defer alloc.free(wbuf);
            const rc = win.GetCurrentDirectoryW(cap, wbuf.ptr);
            if (rc == 0) return win_errno(win.GetLastError());
            if (rc < cap) {
                const utf8 = win_to_utf8(alloc, wbuf[0..rc]) orelse return -22;
                defer alloc.free(utf8);
                const out = room(buffer, length);
                @memcpy(out[0..@min(out.len, utf8.len)], utf8[0..@min(out.len, utf8.len)]);
                return @intCast(utf8.len);
            }
            cap = rc;
        }
    } else {
        var scratch: [4096]u8 = undefined;
        const rc = result(linux.syscall2(.getcwd, @intFromPtr(&scratch), scratch.len));
        if (rc < 0) return rc;
        const size: usize = @intCast(rc - 1);
        const out = room(buffer, length);
        @memcpy(out[0..@min(out.len, size)], scratch[0..@min(out.len, size)]);
        return @intCast(size);
    }
}

export fn env_chdir(path: [*:0]const u8) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const wpath = win_to_utf16(alloc, path) orelse return -22;
        defer alloc.free(wpath);
        if (win.SetCurrentDirectoryW(wpath.ptr) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall1(.chdir, @intFromPtr(path)));
    }
}
