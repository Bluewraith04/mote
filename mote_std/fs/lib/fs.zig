//! The file system functions of the `fs` library: each gives a result or the negated errno.

const std = @import("std");
const builtin = @import("builtin");
const is_windows = builtin.os.tag == .windows;
const linux = std.os.linux;

/// A fault in an external call stops the program without the standard library's report.
pub const panic = std.debug.no_panic;

const eintr: i64 = -4;
const at_fdcwd: usize = @bitCast(@as(isize, -100));
const o_cloexec: usize = 0o2000000;
const o_directory: usize = 0o200000;

const win = struct {
    const HANDLE = ?*anyopaque;
    const INVALID_HANDLE_VALUE: HANDLE = @ptrFromInt(std.math.maxInt(usize));
    const GENERIC_READ: u32 = 0x80000000;
    const GENERIC_WRITE: u32 = 0x40000000;
    const FILE_APPEND_DATA: u32 = 0x00000004;
    const SYNCHRONIZE: u32 = 0x00100000;
    const FILE_SHARE_ALL: u32 = 1 | 2 | 4;
    const CREATE_NEW: u32 = 1;
    const CREATE_ALWAYS: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const OPEN_ALWAYS: u32 = 4;
    const TRUNCATE_EXISTING: u32 = 5;
    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x00200000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000;
    const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000000C;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFFFFFF;

    const ByHandleInfo = extern struct {
        attributes: u32,
        creation_lo: u32,
        creation_hi: u32,
        access_lo: u32,
        access_hi: u32,
        write_lo: u32,
        write_hi: u32,
        volume_serial: u32,
        size_hi: u32,
        size_lo: u32,
        links: u32,
        index_hi: u32,
        index_lo: u32,
    };

    const AttributeTagInfo = extern struct {
        attributes: u32,
        reparse_tag: u32,
    };

    const FindDataW = extern struct {
        attributes: u32,
        creation_lo: u32,
        creation_hi: u32,
        access_lo: u32,
        access_hi: u32,
        write_lo: u32,
        write_hi: u32,
        size_hi: u32,
        size_lo: u32,
        reserved0: u32,
        reserved1: u32,
        file_name: [260]u16,
        alt_name: [14]u16,
    };

    extern "kernel32" fn GetLastError() callconv(.winapi) u32;
    extern "kernel32" fn CreateFileW(lpFileName: [*:0]const u16, dwDesiredAccess: u32, dwShareMode: u32, lpSecurityAttributes: ?*anyopaque, dwCreationDisposition: u32, dwFlagsAndAttributes: u32, hTemplateFile: HANDLE) callconv(.winapi) HANDLE;
    extern "kernel32" fn CloseHandle(hObject: HANDLE) callconv(.winapi) i32;
    extern "kernel32" fn ReadFile(hFile: HANDLE, lpBuffer: [*]u8, nNumberOfBytesToRead: u32, lpNumberOfBytesRead: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn WriteFile(hFile: HANDLE, lpBuffer: [*]const u8, nNumberOfBytesToWrite: u32, lpNumberOfBytesWritten: ?*u32, lpOverlapped: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn SetFilePointerEx(hFile: HANDLE, liDistanceToMove: i64, lpNewFilePointer: ?*i64, dwMoveMethod: u32) callconv(.winapi) i32;
    extern "kernel32" fn SetEndOfFile(hFile: HANDLE) callconv(.winapi) i32;
    extern "kernel32" fn CreateDirectoryW(lpPathName: [*:0]const u16, lpSecurityAttributes: ?*anyopaque) callconv(.winapi) i32;
    extern "kernel32" fn RemoveDirectoryW(lpPathName: [*:0]const u16) callconv(.winapi) i32;
    extern "kernel32" fn DeleteFileW(lpFileName: [*:0]const u16) callconv(.winapi) i32;
    extern "kernel32" fn MoveFileExW(lpExistingFileName: [*:0]const u16, lpNewFileName: [*:0]const u16, dwFlags: u32) callconv(.winapi) i32;
    extern "kernel32" fn GetFileAttributesW(lpFileName: [*:0]const u16) callconv(.winapi) u32;
    extern "kernel32" fn GetFileInformationByHandle(hFile: HANDLE, lpFileInformation: *ByHandleInfo) callconv(.winapi) i32;
    extern "kernel32" fn GetFileInformationByHandleEx(hFile: HANDLE, FileInformationClass: u32, lpFileInformation: *anyopaque, dwBufferSize: u32) callconv(.winapi) i32;
    extern "kernel32" fn FindFirstFileW(lpFileName: [*:0]const u16, lpFindFileData: *FindDataW) callconv(.winapi) HANDLE;
    extern "kernel32" fn FindNextFileW(hFindFile: HANDLE, lpFindFileData: *FindDataW) callconv(.winapi) i32;
    extern "kernel32" fn FindClose(hFindFile: HANDLE) callconv(.winapi) i32;
    extern "kernel32" fn MultiByteToWideChar(CodePage: u32, dwFlags: u32, lpMultiByteStr: [*]const u8, cbMultiByte: i32, lpWideCharStr: ?[*]u16, cchWideChar: i32) callconv(.winapi) i32;
    extern "kernel32" fn WideCharToMultiByte(CodePage: u32, dwFlags: u32, lpWideCharStr: [*]const u16, cchWideChar: i32, lpMultiByteStr: ?[*]u8, cbMultiByte: i32, lpDefaultChar: ?[*:0]const u8, lpUsedDefaultChar: ?*i32) callconv(.winapi) i32;
};

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
        2, 3, 123, 161 => -2, // ENOENT
        5 => -13, // EACCES
        6 => -9, // EBADF
        8, 14 => -12, // ENOMEM
        17, 183 => -17, // EEXIST
        19 => -30, // EROFS
        39, 112 => -28, // ENOSPC
        80 => -17, // EEXIST
        87 => -22, // EINVAL
        109, 232 => -32, // EPIPE
        145 => -39, // ENOTEMPTY
        206 => -36, // ENAMETOOLONG
        267 => -20, // ENOTDIR
        else => -5, // EIO
    };
}

fn win_handle(fd: i64) ?win.HANDLE {
    if (fd < 0) return null;
    const ptr: *anyopaque = @ptrFromInt(@as(usize, @intCast(fd)));
    return ptr;
}

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
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const raw = std.mem.span(path);
        const wpath = win_to_utf16(alloc, raw) orelse return -22;
        defer alloc.free(wpath);
        const acc = flags & 3;
        const create = (flags & 64) != 0;
        const excl = (flags & 128) != 0;
        const trunc = (flags & 512) != 0;
        const append = (flags & 1024) != 0;
        var desired: u32 = 0;
        if (acc == 0 or acc == 2) desired |= win.GENERIC_READ;
        if (append) {
            desired |= win.FILE_APPEND_DATA | win.SYNCHRONIZE;
        } else if (acc == 1 or acc == 2) {
            desired |= win.GENERIC_WRITE;
        }
        const disp: u32 = if (create and excl)
            win.CREATE_NEW
        else if (create and trunc)
            win.CREATE_ALWAYS
        else if (create)
            win.OPEN_ALWAYS
        else if (trunc)
            win.TRUNCATE_EXISTING
        else
            win.OPEN_EXISTING;
        const h = win.CreateFileW(wpath.ptr, desired, win.FILE_SHARE_ALL, null, disp, 0x80, null);
        if (h == win.INVALID_HANDLE_VALUE) {
            const err = win.GetLastError();
            if (err == 5) {
                const attrs = win.GetFileAttributesW(wpath.ptr);
                if (attrs != win.INVALID_FILE_ATTRIBUTES and (attrs & win.FILE_ATTRIBUTE_DIRECTORY) != 0) return -21;
            }
            return win_errno(err);
        }
        if (append) {
            _ = win.SetFilePointerEx(h, 0, null, 2);
        }
        return @intCast(@intFromPtr(h));
    } else {
        return result(linux.syscall4(.openat, at_fdcwd, @intFromPtr(path), @as(usize, @intCast(flags)) | o_cloexec, @intCast(mode)));
    }
}

export fn fs_close(fd: i64) i64 {
    if (is_windows) {
        const h = win_handle(fd) orelse return -9;
        if (win.CloseHandle(h) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall1(.close, fd_of(fd)));
    }
}

export fn fs_read(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const into = slice(buffer, length, count);
    if (is_windows) {
        const h = win_handle(fd) orelse return -9;
        const want: u32 = @intCast(@min(into.len, std.math.maxInt(u32)));
        var got: u32 = 0;
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

/// Writes the first `count` bytes, retrying a short write.
export fn fs_write(fd: i64, buffer: [*]u8, length: i64, count: i64) i64 {
    const all = slice(buffer, length, count);
    if (is_windows) {
        const h = win_handle(fd) orelse return -9;
        var done: usize = 0;
        while (done < all.len) {
            const chunk: u32 = @intCast(@min(all.len - done, std.math.maxInt(u32)));
            var wrote: u32 = 0;
            if (win.WriteFile(h, all.ptr + done, chunk, &wrote, null) == 0) return win_errno(win.GetLastError());
            done += wrote;
        }
        return @intCast(done);
    } else {
        var done: usize = 0;
        while (done < all.len) {
            const rc = result(linux.syscall3(.write, fd_of(fd), @intFromPtr(all.ptr) + done, all.len - done));
            if (rc == eintr) continue;
            if (rc < 0) return rc;
            done += @intCast(rc);
        }
        return @intCast(done);
    }
}

export fn fs_seek(fd: i64, offset: i64, whence: i64) i64 {
    if (is_windows) {
        const h = win_handle(fd) orelse return -9;
        if (whence < 0 or whence > 2) return -22;
        var pos: i64 = 0;
        if (win.SetFilePointerEx(h, offset, &pos, @intCast(whence)) == 0) return win_errno(win.GetLastError());
        return pos;
    } else {
        return result(linux.syscall3(.lseek, fd_of(fd), @bitCast(offset), @intCast(whence)));
    }
}

export fn fs_truncate(fd: i64, size: i64) i64 {
    if (is_windows) {
        const h = win_handle(fd) orelse return -9;
        var current: i64 = 0;
        if (win.SetFilePointerEx(h, 0, &current, 1) == 0) return win_errno(win.GetLastError());
        if (win.SetFilePointerEx(h, size, null, 0) == 0) return win_errno(win.GetLastError());
        if (win.SetEndOfFile(h) == 0) {
            const err = win.GetLastError();
            _ = win.SetFilePointerEx(h, current, null, 0);
            return win_errno(err);
        }
        _ = win.SetFilePointerEx(h, current, null, 0);
        return 0;
    } else {
        return result(linux.syscall2(.ftruncate, fd_of(fd), @bitCast(size)));
    }
}

export fn fs_mkdir(path: [*:0]const u8, mode: i64) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const wpath = win_to_utf16(alloc, std.mem.span(path)) orelse return -22;
        defer alloc.free(wpath);
        if (win.CreateDirectoryW(wpath.ptr, null) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall3(.mkdirat, at_fdcwd, @intFromPtr(path), @intCast(mode)));
    }
}

export fn fs_rmdir(path: [*:0]const u8) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const wpath = win_to_utf16(alloc, std.mem.span(path)) orelse return -22;
        defer alloc.free(wpath);
        if (win.RemoveDirectoryW(wpath.ptr) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall3(.unlinkat, at_fdcwd, @intFromPtr(path), 0x200));
    }
}

export fn fs_unlink(path: [*:0]const u8) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const wpath = win_to_utf16(alloc, std.mem.span(path)) orelse return -22;
        defer alloc.free(wpath);
        const attrs = win.GetFileAttributesW(wpath.ptr);
        if (attrs != win.INVALID_FILE_ATTRIBUTES and (attrs & win.FILE_ATTRIBUTE_DIRECTORY) != 0) {
            if ((attrs & win.FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
                if (win.RemoveDirectoryW(wpath.ptr) == 0) return win_errno(win.GetLastError());
                return 0;
            }
            return -21; // EISDIR
        }
        if (win.DeleteFileW(wpath.ptr) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall3(.unlinkat, at_fdcwd, @intFromPtr(path), 0));
    }
}

export fn fs_rename(from: [*:0]const u8, to: [*:0]const u8) i64 {
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const wfrom = win_to_utf16(alloc, std.mem.span(from)) orelse return -22;
        defer alloc.free(wfrom);
        const wto = win_to_utf16(alloc, std.mem.span(to)) orelse return -22;
        defer alloc.free(wto);
        if (win.MoveFileExW(wfrom.ptr, wto.ptr, win.MOVEFILE_REPLACE_EXISTING | win.MOVEFILE_WRITE_THROUGH) == 0) return win_errno(win.GetLastError());
        return 0;
    } else {
        return result(linux.syscall4(.renameat, at_fdcwd, @intFromPtr(from), at_fdcwd, @intFromPtr(to)));
    }
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

fn fill_win_handle(h: win.HANDLE, out: [*]i64, length: i64, check_symlink: bool) i64 {
    if (length < 4) return -22;
    var info: win.ByHandleInfo = undefined;
    if (win.GetFileInformationByHandle(h, &info) == 0) return win_errno(win.GetLastError());
    var is_sym = false;
    if (check_symlink and (info.attributes & win.FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
        var tag: win.AttributeTagInfo = undefined;
        if (win.GetFileInformationByHandleEx(h, 9, &tag, @sizeOf(win.AttributeTagInfo)) != 0) {
            if (tag.reparse_tag == win.IO_REPARSE_TAG_SYMLINK) is_sym = true;
        }
    }
    if (is_sym) {
        out[0] = 3;
    } else if ((info.attributes & win.FILE_ATTRIBUTE_DIRECTORY) != 0) {
        out[0] = 2;
    } else {
        out[0] = 1;
    }
    const size = (@as(u64, info.size_hi) << 32) | info.size_lo;
    out[1] = @intCast(size);
    const ft = (@as(u64, info.write_hi) << 32) | info.write_lo;
    const epoch_100ns: u64 = 116_444_736_000_000_000;
    const unix_ns: i64 = if (ft >= epoch_100ns) @intCast((ft - epoch_100ns) * 100) else -@as(i64, @intCast((epoch_100ns - ft) * 100));
    out[2] = unix_ns;
    out[3] = if ((info.attributes & win.FILE_ATTRIBUTE_READONLY) != 0) 0o444 else if (out[0] == 2) 0o755 else 0o644;
    return 0;
}

/// `follow` is not 0 to look through a symbolic link.
export fn fs_stat(path: [*:0]const u8, out: [*]i64, length: i64, follow: i64) i64 {
    if (is_windows) {
        if (length < 4) return -22;
        const alloc = std.heap.page_allocator;
        const wpath = win_to_utf16(alloc, std.mem.span(path)) orelse return -22;
        defer alloc.free(wpath);
        var flags: u32 = win.FILE_FLAG_BACKUP_SEMANTICS;
        if (follow == 0) flags |= win.FILE_FLAG_OPEN_REPARSE_POINT;
        const h = win.CreateFileW(wpath.ptr, 0, win.FILE_SHARE_ALL, null, win.OPEN_EXISTING, flags, null);
        if (h == win.INVALID_HANDLE_VALUE) return win_errno(win.GetLastError());
        defer _ = win.CloseHandle(h);
        return fill_win_handle(h, out, length, follow == 0);
    } else {
        var stat: linux.Statx = undefined;
        const flags: usize = if (follow != 0) 0 else 0x100;
        const rc = result(linux.syscall5(.statx, at_fdcwd, @intFromPtr(path), flags, 0x7ff, @intFromPtr(&stat)));
        if (rc < 0) return rc;
        return fill(out, length, &stat);
    }
}

export fn fs_fstat(fd: i64, out: [*]i64, length: i64) i64 {
    if (is_windows) {
        const h = win_handle(fd) orelse return -9;
        return fill_win_handle(h, out, length, false);
    } else {
        var stat: linux.Statx = undefined;
        const empty: [*:0]const u8 = "";
        const rc = result(linux.syscall5(.statx, fd_of(fd), @intFromPtr(empty), 0x1000, 0x7ff, @intFromPtr(&stat)));
        if (rc < 0) return rc;
        return fill(out, length, &stat);
    }
}

/// Writes the names in a directory, each ended by a NUL, into the buffer; gives the size they need, which is more than `length` when they did not fit.
export fn fs_list(path: [*:0]const u8, buffer: [*]u8, length: i64) i64 {
    const room: usize = @intCast(@max(length, 0));
    if (is_windows) {
        const alloc = std.heap.page_allocator;
        const raw = std.mem.span(path);
        const wpath = win_to_utf16(alloc, raw) orelse return -22;
        defer alloc.free(wpath);
        const attrs = win.GetFileAttributesW(wpath.ptr);
        if (attrs == win.INVALID_FILE_ATTRIBUTES) return win_errno(win.GetLastError());
        if ((attrs & win.FILE_ATTRIBUTE_DIRECTORY) == 0) return -20; // ENOTDIR
        const pattern = if (raw.len > 0 and (raw[raw.len - 1] == '\\' or raw[raw.len - 1] == '/'))
            std.fmt.allocPrint(alloc, "{s}*", .{raw}) catch return -12
        else
            std.fmt.allocPrint(alloc, "{s}\\*", .{raw}) catch return -12;
        defer alloc.free(pattern);
        const wpat = win_to_utf16(alloc, pattern) orelse return -22;
        defer alloc.free(wpat);
        var data: win.FindDataW = undefined;
        const find = win.FindFirstFileW(wpat.ptr, &data);
        if (find == win.INVALID_HANDLE_VALUE) {
            const err = win.GetLastError();
            if (err == 2 or err == 18) return 0; // empty dir
            return win_errno(err);
        }
        defer _ = win.FindClose(find);
        var needed: usize = 0;
        while (true) {
            var name_len: usize = 0;
            while (name_len < data.file_name.len and data.file_name[name_len] != 0) : (name_len += 1) {}
            const utf8 = win_to_utf8(alloc, data.file_name[0..name_len]) orelse return -22;
            defer alloc.free(utf8);
            if (!std.mem.eql(u8, utf8, ".") and !std.mem.eql(u8, utf8, "..")) {
                if (needed + utf8.len + 1 <= room) {
                    @memcpy(buffer[needed..][0..utf8.len], utf8);
                    buffer[needed + utf8.len] = 0;
                }
                needed += utf8.len + 1;
            }
            if (win.FindNextFileW(find, &data) == 0) break;
        }
        return @intCast(needed);
    } else {
        const fd = result(linux.syscall4(.openat, at_fdcwd, @intFromPtr(path), o_directory | o_cloexec, 0));
        if (fd < 0) return fd;
        defer _ = linux.syscall1(.close, fd_of(fd));
        var chunk: [4096]u8 align(8) = undefined;
        var needed: usize = 0;
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
}
