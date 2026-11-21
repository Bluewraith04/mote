//! The functions compiled code calls besides the runtime entries, bound by name in the JIT.

use mote_abi::{Meta, TaskContext, TypeId, ELEMENTS_OFFSET, LENGTH_OFFSET, LIST_BUFFER_OFFSET, LIST_CAPACITY_OFFSET};

use crate::show::{float_text, Show};
use crate::walk::{Copier, Walk};
use crate::Runtime;

/// The symbol that prints a scalar or a string.
pub const PRINT: &str = "mote_print";

/// The symbol of the remainder of two `Float` values.
pub const FMOD: &str = "mote_fmod";

/// The symbol that writes the text of a scalar into a buffer.
pub const SHOW: &str = "mote_show";

/// The symbol that compares two strings.
pub const COMPARE: &str = "mote_string_compare";

/// The symbol that tells whether a cut of a string keeps its characters whole.
pub const CUT: &str = "mote_string_cut";

/// The symbol of the string operations.
pub const TEXT: &str = "mote_text";

/// The symbol of the `Float` functions the machine has no instruction for.
pub const MATH: &str = "mote_math";

/// The symbol of off-heap memory operations (`alloc`, `realloc`, `free`, `copy`, `zero`).
pub const MEM: &str = "mote_mem";

/// The codes `mote_mem` is given.
pub mod mem {
    pub const ALLOC: u32 = 0;
    pub const REALLOC: u32 = 1;
    pub const FREE: u32 = 2;
    pub const COPY: u32 = 3;
    pub const ZERO: u32 = 4;
}

/// The codes `mote_math` is given.
pub mod math {
    pub const ROUND: u32 = 0;
    pub const EXP: u32 = 1;
    pub const LN: u32 = 2;
    pub const LOG10: u32 = 3;
    pub const SIN: u32 = 4;
    pub const COS: u32 = 5;
    pub const TAN: u32 = 6;
    pub const POW: u32 = 7;
    pub const ATAN2: u32 = 8;
}

/// Bytes a buffer for `mote_show` holds; the longest text of a `Float` is under this.
pub const SHOW_BUFFER: u32 = 400;

/// How `mote_print` and `mote_show` read the bits they are given.
pub mod shown {
    pub const SIGNED: u32 = 0;
    pub const UNSIGNED: u32 = 1;
    pub const BOOL: u32 = 2;
    pub const FLOAT: u32 = 3;
    pub const FLOAT32: u32 = 4;
    /// The bits are a reference to a string object.
    pub const STRING: u32 = 5;
}

/// The bytes of a string object.
///
/// # Safety
/// `reference` is the address of a live string object.
unsafe fn string_bytes<'a>(reference: u64) -> &'a [u8] {
    let object = reference as usize;
    // SAFETY: the caller's promise: a string holds its byte length at `LENGTH_OFFSET` and the bytes from `ELEMENTS_OFFSET`.
    unsafe {
        let length = *((object + LENGTH_OFFSET as usize) as *const u64) as usize;
        std::slice::from_raw_parts((object + ELEMENTS_OFFSET as usize) as *const u8, length)
    }
}

/// The text of a scalar as `print` shows it.
fn scalar_text(bits: u64, kind: u32) -> String {
    match kind {
        shown::SIGNED => (bits as i64).to_string(),
        shown::UNSIGNED => bits.to_string(),
        shown::BOOL => (bits != 0).to_string(),
        shown::FLOAT32 => float_text(f64::from_bits(bits), true),
        _ => float_text(f64::from_bits(bits), false),
    }
}

/// The flags of a print: a newline follows the text.
pub const PRINT_NEWLINE: u64 = 1;
/// The flags of a print: the text goes to standard error, not to the output.
pub const PRINT_ERROR: u64 = 2;

fn write_printed(runtime: &Runtime, bytes: &[u8], flags: u64) {
    if flags & PRINT_ERROR != 0 {
        runtime.write_error(bytes);
    } else {
        runtime.write_output(bytes);
    }
}

/// Adds the text of a scalar or a string to the output, or to standard error; `flags` is `PRINT_NEWLINE` and `PRINT_ERROR`.
#[unsafe(no_mangle)]
pub extern "C" fn mote_print(context: *mut TaskContext, bits: u64, kind: u32, flags: u32) {
    let mut bytes = if kind == shown::STRING {
        // SAFETY: compiled code passes the reference of a string it holds.
        unsafe { string_bytes(bits) }.to_vec()
    } else {
        scalar_text(bits, kind).into_bytes()
    };
    if u64::from(flags) & PRINT_NEWLINE != 0 {
        bytes.push(b'\n');
    }
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    write_printed(unsafe { Runtime::of(context) }, &bytes, u64::from(flags));
}

/// The symbol of the operations on the process: its input, arguments, end and clock.
pub const SYSTEM: &str = "mote_system";

/// The codes `mote_system` is given.
pub mod system {
    /// The next line of standard input as a string of type `t1` without its line break; `u64::MAX` at the end of the input, 0 when the heap is full.
    pub const READ_LINE: u32 = 0;
    /// The arguments of the program as a list of strings: `t1` is the list type, `t2` its buffer and `t3` the string; 0 when the heap is full.
    pub const ARGS: u32 = 1;
    /// Ends the program with the exit code `a`; does not return.
    pub const EXIT: u32 = 2;
    /// The nanoseconds on the scheduler's clock.
    pub const NOW: u32 = 3;
}

/// An operation on the process by its code in `system`.
#[unsafe(no_mangle)]
pub extern "C" fn mote_system(context: *mut TaskContext, op: u32, a: u64, t1: u32, t2: u32, t3: u32) -> u64 {
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    let runtime = unsafe { Runtime::of(context) };
    match op {
        system::READ_LINE => {
            let mut line = String::new();
            runtime.flush_sink();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => u64::MAX,
                Ok(_) => {
                    let text = line.strip_suffix('\n').map_or(line.as_str(), |text| text.strip_suffix('\r').unwrap_or(text));
                    // SAFETY: the context is attached and `t1` is the string type.
                    unsafe { new_string(context, t1, text.as_bytes()) }
                }
            }
        }
        system::ARGS => {
            let args = runtime.program_args();
            // SAFETY: the type ids are those of the list, its buffer and the string, which the caller passes.
            unsafe {
                let buffer = new_object(context, t2, args.len() as u64);
                if buffer == 0 {
                    return 0;
                }
                for (at, arg) in args.iter().enumerate() {
                    let string = new_string(context, t3, arg.as_bytes());
                    if string == 0 {
                        return 0;
                    }
                    set_word(buffer, ELEMENTS_OFFSET + 8 * at as u32, string);
                }
                let list = new_object(context, t1, 0);
                if list != 0 {
                    set_word(list, LENGTH_OFFSET, args.len() as u64);
                    set_word(list, LIST_CAPACITY_OFFSET, args.len() as u64);
                    set_word(list, LIST_BUFFER_OFFSET, buffer);
                }
                list
            }
        }
        system::EXIT => runtime.exit(context, a as i32),
        system::NOW => {
            if mote_scheduler::in_a_task() {
                mote_scheduler::now()
            } else {
                runtime.monotonic_now()
            }
        }
        _ => 0,
    }
}

/// Allocates a SIMD-aligned (`>= 64` bytes) off-heap block prefixed by its `(raw_ptr, total_size, align)` header.
fn mem_alloc(count: u64, stride: u64, align: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let Ok(count) = usize::try_from(count) else { return 0 };
    let Ok(stride) = usize::try_from(stride) else { return 0 };
    let Ok(align) = usize::try_from(align) else { return 0 };
    let user_align = align.max(64).next_power_of_two();
    let Some(data_bytes) = count.checked_mul(stride) else { return 0 };
    let Some(total_size) = user_align.checked_add(data_bytes.max(1)) else { return 0 };
    let Ok(layout) = std::alloc::Layout::from_size_align(total_size, user_align) else { return 0 };
    // SAFETY: `layout` has non-zero `total_size` and power-of-two `user_align`.
    let raw = unsafe { std::alloc::alloc(layout) };
    if raw.is_null() {
        return 0;
    }
    // SAFETY: `raw` is followed by `total_size >= user_align >= 64 >= 3 * size_of::<usize>()` bytes.
    unsafe {
        let user_ptr = raw.add(user_align);
        let header = user_ptr.cast::<usize>();
        header.sub(3).write(raw as usize);
        header.sub(2).write(total_size);
        header.sub(1).write(user_align);
        user_ptr as u64
    }
}

/// Frees an off-heap allocation previously returned by `mem_alloc`.
fn mem_free(ptr: u64) {
    if ptr == 0 {
        return;
    }
    let header = ptr as usize as *const usize;
    // SAFETY: non-null pointers from `mem_alloc` store `[raw, total_size, user_align]` in the 3 preceding words.
    unsafe {
        let raw = header.sub(3).read() as *mut u8;
        let total_size = header.sub(2).read();
        let user_align = header.sub(1).read();
        let layout = std::alloc::Layout::from_size_align_unchecked(total_size, user_align);
        std::alloc::dealloc(raw, layout);
    }
}

/// Reallocates an off-heap allocation to hold `new_count` elements while preserving up to the overlapping byte length.
fn mem_realloc(old_ptr: u64, new_count: u64, stride: u64, align: u64) -> u64 {
    if new_count == 0 {
        mem_free(old_ptr);
        return 0;
    }
    if old_ptr == 0 {
        return mem_alloc(new_count, stride, align);
    }
    let Ok(new_count_usize) = usize::try_from(new_count) else { return 0 };
    let Ok(stride_usize) = usize::try_from(stride) else { return 0 };
    let Some(new_data) = new_count_usize.checked_mul(stride_usize) else { return 0 };
    let new_ptr = mem_alloc(new_count, stride, align);
    if new_ptr == 0 {
        return 0;
    }
    let old_header = old_ptr as usize as *const usize;
    // SAFETY: `old_ptr` came from `mem_alloc` and `new_ptr` was just allocated with at least `new_data` bytes.
    unsafe {
        let old_raw = old_header.sub(3).read() as *mut u8;
        let old_total = old_header.sub(2).read();
        let old_align = old_header.sub(1).read();
        let old_data = old_total - old_align;
        let copy_bytes = old_data.min(new_data);
        if copy_bytes > 0 {
            std::ptr::copy_nonoverlapping(old_ptr as usize as *const u8, new_ptr as usize as *mut u8, copy_bytes);
        }
        std::alloc::dealloc(old_raw, std::alloc::Layout::from_size_align_unchecked(old_total, old_align));
    }
    new_ptr
}

/// Performs an off-heap allocation, reallocation, free, copy, or zero operation by its code in `mem`.
#[unsafe(no_mangle)]
pub extern "C" fn mote_mem(op: u32, a: u64, b: u64, c: u64, d: u64) -> u64 {
    match op {
        mem::ALLOC => mem_alloc(a, b, c),
        mem::REALLOC => mem_realloc(a, b, c, d),
        mem::FREE => {
            mem_free(a);
            0
        }
        mem::COPY => {
            let (Ok(count), Ok(stride)) = (usize::try_from(c), usize::try_from(d)) else { return 1 };
            let Some(bytes) = count.checked_mul(stride) else { return 1 };
            if bytes > 0 {
                // SAFETY: caller validates non-null dst (`a`) and src (`b`) covering `bytes` bytes.
                unsafe { std::ptr::copy(b as usize as *const u8, a as usize as *mut u8, bytes) };
            }
            0
        }
        mem::ZERO => {
            let (Ok(count), Ok(stride)) = (usize::try_from(b), usize::try_from(c)) else { return 1 };
            let Some(bytes) = count.checked_mul(stride) else { return 1 };
            if bytes > 0 {
                // SAFETY: caller validates non-null dst (`a`) covering `bytes` bytes.
                unsafe { std::ptr::write_bytes(a as usize as *mut u8, 0, bytes) };
            }
            0
        }
        _ => 0,
    }
}

/// Writes the text of a scalar at `buffer`, which holds `SHOW_BUFFER` bytes, and gives its length.
#[unsafe(no_mangle)]
pub extern "C" fn mote_show(bits: u64, kind: u32, buffer: *mut u8) -> u64 {
    let text = scalar_text(bits, kind);
    assert!(text.len() <= SHOW_BUFFER as usize, "the text of a number fits the buffer");
    // SAFETY: compiled code passes a buffer of `SHOW_BUFFER` bytes.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buffer, text.len()) };
    text.len() as u64
}

/// Orders two strings by their bytes: -1, 0 or 1.
#[unsafe(no_mangle)]
pub extern "C" fn mote_string_compare(left: u64, right: u64) -> i64 {
    // SAFETY: compiled code passes the references of two strings it holds.
    let (left, right) = unsafe { (string_bytes(left), string_bytes(right)) };
    left.cmp(right) as i64
}

/// Whether `start..end` of a string cuts between characters, as 1 or 0.
#[unsafe(no_mangle)]
pub extern "C" fn mote_string_cut(string: u64, start: u64, end: u64) -> u32 {
    // SAFETY: compiled code passes the reference of a string it holds.
    let bytes = unsafe { string_bytes(string) };
    // SAFETY: a string object holds UTF-8.
    let text = unsafe { std::str::from_utf8_unchecked(bytes) };
    u32::from(text.get(start as usize..end as usize).is_some())
}

/// The most bytes `repeat` makes, which is 64 MiB.
const TEXT_LIMIT: u64 = 64 * 1024 * 1024;

/// A new string object of the bytes, or 0 when the heap is full.
///
/// # Safety
/// `context` is attached to a live runtime and `string` is its string type.
pub(crate) unsafe fn new_string(context: *mut TaskContext, string: u32, bytes: &[u8]) -> u64 {
    // SAFETY: the caller's promise.
    let object = unsafe { new_object(context, string, bytes.len() as u64) };
    if object != 0 {
        // SAFETY: a string holds its bytes from `ELEMENTS_OFFSET`, and the object was allocated for `bytes.len()` of them.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), (object as usize + ELEMENTS_OFFSET as usize) as *mut u8, bytes.len()) };
    }
    object
}

/// A new zeroed object of the type with room for `count` elements, or 0 when the heap is full.
///
/// # Safety
/// `context` is attached to a live runtime.
pub(crate) unsafe fn new_object(context: *mut TaskContext, ty: u32, count: u64) -> u64 {
    // SAFETY: the caller's promise.
    let (runtime, context) = unsafe { (Runtime::of(context), &mut *context) };
    runtime.alloc(context, TypeId::from_raw(ty), count).map_or(0, |object| object.address() as u64)
}

/// Writes a word of an object.
///
/// # Safety
/// `object` is the address of a live object with a word at `offset`.
pub(crate) unsafe fn set_word(object: u64, offset: u32, word: u64) {
    // SAFETY: the caller's promise.
    unsafe { *((object as usize + offset as usize) as *mut u64) = word };
}

/// The text operations of strings, by code, for `mote_text`.
pub mod text {
    pub const REPLACE: u32 = 0;
    pub const UPPER: u32 = 1;
    pub const LOWER: u32 = 2;
    pub const TRIM: u32 = 3;
    pub const REPEAT: u32 = 4;
    pub const FIND: u32 = 5;
    pub const RFIND: u32 = 6;
    pub const SPLIT: u32 = 7;
    pub const BYTES: u32 = 8;
    pub const VALID: u32 = 9;
    pub const FROM_BYTES: u32 = 10;
    pub const PARSE_INT: u32 = 11;
    pub const PARSE_FLOAT: u32 = 12;
}

/// A string operation by its code in `text`; the result is a string, a position (-1 for none), a flag, or 0 when the heap is full.
///
/// `a`, `b` and `c` are the operands, `t1`..`t3` the type ids the result is built from, and `c` is where a parse writes its number.
#[unsafe(no_mangle)]
pub extern "C" fn mote_text(context: *mut TaskContext, op: u32, a: u64, b: u64, c: u64, t1: u32, t2: u32, t3: u32) -> u64 {
    // SAFETY: compiled code passes the references of strings it holds, and a string object holds UTF-8.
    let s = || unsafe { std::str::from_utf8_unchecked(string_bytes(a)) };
    // SAFETY: as above.
    let other = || unsafe { std::str::from_utf8_unchecked(string_bytes(b)) };
    // SAFETY: the context is attached and `t1` is the string type for the operations that call this.
    let make = |text: &str| unsafe { new_string(context, t1, text.as_bytes()) };
    match op {
        text::REPLACE if other().is_empty() => a,
        // SAFETY: as above.
        text::REPLACE => make(&s().replace(other(), unsafe { std::str::from_utf8_unchecked(string_bytes(c)) })),
        text::UPPER => make(&s().to_uppercase()),
        text::LOWER => make(&s().to_lowercase()),
        text::TRIM => make(s().trim()),
        text::REPEAT => match (s().len() as u64).checked_mul(b) {
            Some(total) if total <= TEXT_LIMIT => make(&s().repeat(b as usize)),
            _ => 0,
        },
        text::FIND => s().find(other()).map_or(u64::MAX, |at| at as u64),
        text::RFIND => s().rfind(other()).map_or(u64::MAX, |at| at as u64),
        text::SPLIT => {
            let whole = s();
            let parts: Vec<&str> = match other() {
                "" => whole.char_indices().map(|(at, c)| &whole[at..at + c.len_utf8()]).collect(),
                separator => whole.split(separator).collect(),
            };
            // SAFETY: the type ids are those of the list, its buffer and the string, which the caller passes.
            unsafe {
                let buffer = new_object(context, t2, parts.len() as u64);
                if buffer == 0 {
                    return 0;
                }
                for (at, part) in parts.iter().enumerate() {
                    let string = new_string(context, t3, part.as_bytes());
                    if string == 0 {
                        return 0;
                    }
                    set_word(buffer, ELEMENTS_OFFSET + 8 * at as u32, string);
                }
                let list = new_object(context, t1, 0);
                if list != 0 {
                    set_word(list, LENGTH_OFFSET, parts.len() as u64);
                    set_word(list, LIST_CAPACITY_OFFSET, parts.len() as u64);
                    set_word(list, LIST_BUFFER_OFFSET, buffer);
                }
                list
            }
        }
        text::BYTES => {
            let bytes = s().as_bytes();
            // SAFETY: `t1` is the type of a `[UInt8]`.
            let array = unsafe { new_object(context, t1, bytes.len() as u64) };
            if array != 0 {
                // SAFETY: the array holds `bytes.len()` bytes from `ELEMENTS_OFFSET`.
                unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), (array as usize + ELEMENTS_OFFSET as usize) as *mut u8, bytes.len()) };
            }
            array
        }
        // SAFETY: `a` is the reference of a `[UInt8]`, which holds its length at `LENGTH_OFFSET` and its bytes from `ELEMENTS_OFFSET`, as a string does.
        text::VALID => u64::from(std::str::from_utf8(unsafe { string_bytes(a) }).is_ok()),
        // SAFETY: as above, and the caller checked with `VALID`.
        text::FROM_BYTES => make(unsafe { std::str::from_utf8_unchecked(string_bytes(a)) }),
        text::PARSE_INT => s().parse::<i64>().map_or(0, |n| {
            // SAFETY: `c` is the address of a stack word.
            unsafe { *(c as *mut u64) = n as u64 };
            1
        }),
        text::PARSE_FLOAT => s().parse::<f64>().map_or(0, |n| {
            // SAFETY: as above.
            unsafe { *(c as *mut u64) = n.to_bits() };
            1
        }),
        _ => 0,
    }
}

/// The value operations, by code, for `mote_value`.
pub mod value {
    pub const EQUAL: u32 = 0;
    pub const KEY_EQ: u32 = 1;
    pub const HASH: u32 = 2;
    pub const CONTAINS: u32 = 3;
    pub const DEEP_CLONE: u32 = 4;
    /// Prints the value at `a`, with a newline when `b` is not 0.
    pub const PRINT: u32 = 5;
    /// A new string of the text of the value at `a`.
    pub const SHOW: u32 = 6;
}

/// The symbol of the operations on values of any type.
pub const VALUE: &str = "mote_value";

/// An operation on values of the type `ty` by its code in `value`.
///
/// `a` and `b` are addresses of values; for `CONTAINS`, `ty` is the array or list and `a` is its reference. Gives a flag, a hash, or for `DEEP_CLONE` 1 when the copy was made and 0 when the heap is full.
#[unsafe(no_mangle)]
pub extern "C" fn mote_value(context: *mut TaskContext, op: u32, ty: u32, a: u64, b: u64) -> u64 {
    // SAFETY: compiled code passes the context it was given, which was attached to a live runtime.
    let runtime = unsafe { Runtime::of(context) };
    let table = runtime.heap().table();
    let shown = &runtime.data::<Meta>().expect("a compiled program keeps its meta").shown;
    let ty = TypeId::from_raw(ty);
    // SAFETY: compiled code passes addresses of values of the type, or the reference of a live array or list and the address of an element value.
    unsafe {
        match op {
            value::EQUAL => u64::from(Walk::new(table, shown, false).same(ty, a as *const u8, b as *const u8)),
            value::KEY_EQ => u64::from(Walk::new(table, shown, true).same(ty, a as *const u8, b as *const u8)),
            value::HASH => Walk::new(table, shown, true).hash_of(ty, a as *const u8),
            value::CONTAINS => {
                let (element, base, length) = match table.get(ty).kind {
                    mote_abi::Kind::Array(element) => (element, a as usize + ELEMENTS_OFFSET as usize, *((a as usize + LENGTH_OFFSET as usize) as *const u64)),
                    mote_abi::Kind::List(element) => (
                        element,
                        *((a as usize + LIST_BUFFER_OFFSET as usize) as *const u64) as usize + ELEMENTS_OFFSET as usize,
                        *((a as usize + LENGTH_OFFSET as usize) as *const u64),
                    ),
                    mote_abi::Kind::FixedArray(element, count) => (element, a as usize, u64::from(count)),
                    _ => return 0,
                };
                let layout = table.layout(element);
                let stride = mote_abi::align_up(layout.size, layout.align) as usize;
                let walk = Walk::new(table, shown, false);
                u64::from((0..length as usize).any(|at| walk.same(element, (base + at * stride) as *const u8, b as *const u8)))
            }
            value::DEEP_CLONE => u64::from(Copier::new(table, shown, context).copy(ty, a as *const u8, b as *mut u8).is_some()),
            value::PRINT | value::SHOW => {
                let meta = runtime.data::<Meta>().expect("a compiled program keeps its meta");
                let mut text = Show { table, meta }.show(ty, a as *const u8, true);
                if op == value::PRINT {
                    if b & PRINT_NEWLINE != 0 {
                        text.push('\n');
                    }
                    write_printed(runtime, text.as_bytes(), b);
                    return 1;
                }
                new_string(context, meta.string.raw(), text.as_bytes())
            }
            _ => 0,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn mote_fmod(left: f64, right: f64) -> f64 {
    left % right
}

/// A function of `Float` that the machine has no instruction for, by its code in `math`.
#[unsafe(no_mangle)]
pub extern "C" fn mote_math(code: u32, x: f64, y: f64) -> f64 {
    match code {
        math::ROUND => x.round(),
        math::EXP => x.exp(),
        math::LN => x.ln(),
        math::LOG10 => x.log10(),
        math::SIN => x.sin(),
        math::COS => x.cos(),
        math::TAN => x.tan(),
        math::POW => x.powf(y),
        math::ATAN2 => x.atan2(y),
        _ => f64::NAN,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mote_abi::{FaultTable, TypeTable};
    use crate::HeapConfig;

    use super::*;

    fn printed(calls: &[(u64, u32, u32)]) -> String {
        let runtime = Runtime::new(Arc::new(TypeTable::new()), FaultTable::new(), HeapConfig::default());
        let mut context = TaskContext::default();
        runtime.attach(&mut context);
        for &(bits, kind, newline) in calls {
            mote_print(&mut context, bits, kind, newline);
        }
        String::from_utf8(runtime.take_output()).unwrap()
    }

    /// The words of a string object: a header, the byte length, then the bytes.
    fn string(text: &str) -> Vec<u64> {
        let mut words = vec![0, text.len() as u64];
        words.extend(text.as_bytes().chunks(8).map(|chunk| {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            u64::from_le_bytes(word)
        }));
        words
    }

    #[test]
    fn helpers_a_scalar_prints_as_its_text() {
        let text = printed(&[
            ((-5i64) as u64, shown::SIGNED, 1),
            (u64::MAX, shown::UNSIGNED, 1),
            (1, shown::BOOL, 1),
            (0, shown::BOOL, 0),
            (3.0f64.to_bits(), shown::FLOAT, 1),
            (f64::from(0.1f32).to_bits(), shown::FLOAT32, 1),
        ]);
        assert_eq!(text, "-5\n18446744073709551615\ntrue\nfalse3.0\n0.1\n");
    }

    #[test]
    fn helpers_a_string_prints_its_bytes() {
        let hello = string("héllo, world");
        assert_eq!(printed(&[(hello.as_ptr() as u64, shown::STRING, 1)]), "héllo, world\n");
    }

    #[test]
    fn helpers_the_text_of_a_scalar_is_written_to_a_buffer_and_the_longest_fits() {
        let mut buffer = [0u8; SHOW_BUFFER as usize];
        for (bits, kind, want) in [(42u64, shown::SIGNED, "42"), (1, shown::BOOL, "true"), (2.5f64.to_bits(), shown::FLOAT, "2.5")] {
            let length = mote_show(bits, kind, buffer.as_mut_ptr());
            assert_eq!(&buffer[..length as usize], want.as_bytes());
        }
        for extreme in [f64::MAX, f64::MIN_POSITIVE, 5e-324, -1.0e300] {
            assert!(mote_show(extreme.to_bits(), shown::FLOAT, buffer.as_mut_ptr()) <= u64::from(SHOW_BUFFER));
        }
    }

    #[test]
    fn helpers_a_cut_inside_a_character_is_refused() {
        let text = string("aé");
        let at = text.as_ptr() as u64;
        assert_eq!(mote_string_cut(at, 0, 1), 1);
        assert_eq!(mote_string_cut(at, 0, 2), 0);
        assert_eq!(mote_string_cut(at, 1, 3), 1);
        assert_eq!(mote_string_cut(at, 2, 3), 0);
        assert_eq!(mote_string_cut(at, 0, 4), 0);
    }

    #[test]
    fn helpers_strings_compare_by_their_bytes() {
        let (a, b, empty) = (string("abc"), string("abd"), string(""));
        let at = |words: &Vec<u64>| words.as_ptr() as u64;
        assert_eq!(mote_string_compare(at(&a), at(&b)), -1);
        assert_eq!(mote_string_compare(at(&b), at(&a)), 1);
        assert_eq!(mote_string_compare(at(&a), at(&a)), 0);
        assert_eq!(mote_string_compare(at(&empty), at(&a)), -1);
    }

    #[test]
    fn helpers_bytes_that_are_not_utf8_are_refused_and_valid_ones_make_a_string() {
        let mut table = TypeTable::new();
        let string_type = table.add("String", mote_abi::Kind::String);
        let runtime = Runtime::new(Arc::new(table), FaultTable::new(), HeapConfig::default());
        let mut context = TaskContext::default();
        runtime.attach(&mut context);
        let (bad, good) = (vec![0, 1, 0xff], string("é"));
        let ask = |op, words: &Vec<u64>, context: &mut TaskContext| mote_text(context, op, words.as_ptr() as u64, 0, 0, string_type.raw(), 0, 0);
        assert_eq!(ask(text::VALID, &bad, &mut context), 0);
        assert_eq!(ask(text::VALID, &good, &mut context), 1);
        let made = ask(text::FROM_BYTES, &good, &mut context);
        assert_ne!(made, 0);
        // SAFETY: `made` is a string object the call allocated.
        assert_eq!(unsafe { string_bytes(made) }, "é".as_bytes());
    }

    #[test]
    fn helpers_mem_allocates_64_byte_aligned_blocks_reallocs_copies_zeroes_and_frees() {
        assert_eq!(mote_mem(mem::ALLOC, 0, 8, 8, 0), 0);
        mote_mem(mem::FREE, 0, 0, 0, 0);

        let p = mote_mem(mem::ALLOC, 4, 8, 8, 0);
        assert_ne!(p, 0);
        assert_eq!(p % 64, 0);

        // SAFETY: `p` holds 4 `u64` words.
        unsafe {
            let slice = std::slice::from_raw_parts_mut(p as *mut u64, 4);
            slice.copy_from_slice(&[10, 20, 30, 40]);
        }

        let grown = mote_mem(mem::REALLOC, p, 8, 8, 8);
        assert_ne!(grown, 0);
        assert_eq!(grown % 64, 0);
        // SAFETY: `grown` preserves the first 4 `u64` words and has room for 8.
        unsafe {
            assert_eq!(std::slice::from_raw_parts(grown as *const u64, 4), &[10, 20, 30, 40]);
            std::slice::from_raw_parts_mut(grown as *mut u64, 8)[4..].copy_from_slice(&[50, 60, 70, 80]);
            assert_eq!(mote_mem(mem::COPY, (grown as *mut u64).add(2) as u64, grown, 4, 8), 0);
            assert_eq!(std::slice::from_raw_parts(grown as *const u64, 6), &[10, 20, 10, 20, 30, 40]);
            assert_eq!(mote_mem(mem::ZERO, (grown as *mut u64).add(1) as u64, 3, 8, 0), 0);
            assert_eq!(std::slice::from_raw_parts(grown as *const u64, 5), &[10, 0, 0, 0, 30]);
        }

        let shrunk = mote_mem(mem::REALLOC, grown, 2, 8, 8);
        assert_ne!(shrunk, 0);
        assert_eq!(shrunk % 64, 0);
        // SAFETY: `shrunk` keeps the first 2 `u64` words.
        unsafe { assert_eq!(std::slice::from_raw_parts(shrunk as *const u64, 2), &[10, 0]) };

        assert_eq!(mote_mem(mem::REALLOC, shrunk, 0, 8, 8), 0);
    }
}
