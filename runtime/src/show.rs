//! The text `print` writes for a value in memory, read by the type table and the names in `Meta`.

use mote_abi::{Carries, Kind, Meta, PairPayload, Scalar, Shown, TypeId, TypeTable, BOX_VALUE_OFFSET, ELEMENTS_OFFSET, LENGTH_OFFSET, LIST_BUFFER_OFFSET};

/// A `Float` or `Float32` as text, always with a point or an exponent.
pub(crate) fn float_text(x: f64, single: bool) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let text = if single { (x as f32).to_string() } else { x.to_string() };
    if text.contains(['.', 'e']) { text } else { format!("{text}.0") }
}

/// Reads a word of memory of the given width as 64 bits, zero extended.
///
/// # Safety
/// `at` addresses `bytes` readable bytes.
unsafe fn read(at: *const u8, bytes: u32) -> u64 {
    // SAFETY: the caller's promise.
    unsafe {
        match bytes {
            1 => u64::from(*at),
            2 => u64::from((at as *const u16).read_unaligned()),
            4 => u64::from((at as *const u32).read_unaligned()),
            _ => (at as *const u64).read_unaligned(),
        }
    }
}

/// The bytes of a string object.
///
/// # Safety
/// `object` addresses a live string.
unsafe fn text<'a>(object: usize) -> &'a str {
    // SAFETY: the caller's promise: a string holds its length at `LENGTH_OFFSET` and UTF-8 from `ELEMENTS_OFFSET`.
    unsafe {
        let length = *((object + LENGTH_OFFSET as usize) as *const u64) as usize;
        std::str::from_utf8_unchecked(std::slice::from_raw_parts((object + ELEMENTS_OFFSET as usize) as *const u8, length))
    }
}

/// Writes values as `print` does.
pub(crate) struct Show<'a> {
    pub table: &'a TypeTable,
    pub meta: &'a Meta,
}

impl Show<'_> {
    /// The text of the value of the type at the address; a string is bare at the top and quoted inside anything else.
    ///
    /// # Safety
    /// The address holds a value of the type, with the objects it refers to live.
    pub(crate) unsafe fn show(&self, ty: TypeId, at: *const u8, top: bool) -> String {
        let mut out = String::new();
        // SAFETY: the caller's promise.
        unsafe { self.write(&mut out, ty, at, top) };
        out
    }

    unsafe fn write(&self, out: &mut String, ty: TypeId, at: *const u8, top: bool) {
        let info = self.table.get(ty);
        let shown = &self.meta.shown[ty.index()];
        // SAFETY: the caller's promise, and each branch reads only what the layout of the kind holds.
        unsafe {
            match (&info.kind, shown) {
                (Kind::Unit, _) => out.push_str("null"),
                (Kind::Scalar(scalar), shown) => self.scalar(out, *scalar, shown, at),
                (Kind::Struct(_), Shown::Tuple) => {
                    out.push('(');
                    for (index, field) in info.fields.iter().enumerate() {
                        if index > 0 {
                            out.push_str(", ");
                        }
                        self.write(out, field.ty, at.add(field.offset as usize), false);
                    }
                    out.push(')');
                }
                (Kind::Struct(_), Shown::Range) => {
                    let (start, end, inclusive) = (info.fields[0], info.fields[1], info.fields[2]);
                    self.write(out, start.ty, at.add(start.offset as usize), false);
                    out.push_str(if read(at.add(inclusive.offset as usize), 1) != 0 { "..=" } else { ".." });
                    self.write(out, end.ty, at.add(end.offset as usize), false);
                }
                (Kind::Struct(_), Shown::Record(name, names)) => self.record(out, name, names, info.fields.iter().map(|field| (field.ty, at.add(field.offset as usize)))),
                (Kind::FixedArray(element, count), _) => {
                    let layout = self.table.layout(*element);
                    let stride = mote_abi::align_up(layout.size, layout.align) as usize;
                    self.elements(out, *element, (0..*count as usize).map(|index| at.add(index * stride)));
                }
                (Kind::Enum(_), Shown::Enum(variants)) => {
                    let tag = read(at, info.tag_size) as usize;
                    let (name, carries, names) = &variants[tag];
                    out.push_str(name);
                    if *carries != Carries::Nothing {
                        let fields = info.variants[tag].fields.iter().map(|field| (field.ty, at.add(field.offset as usize)));
                        out.push('(');
                        self.fields(out, names, fields);
                        out.push(')');
                    }
                }
                (Kind::Optional(inner), _) if info.is_reference() => self.object(out, *inner, read(at, 8) as usize, top),
                (Kind::Optional(_), _) => {
                    if read(at, 8) == 0 {
                        out.push_str("null");
                    } else {
                        let field = info.variants[1].fields[0];
                        self.write(out, field.ty, at.add(field.offset as usize), top);
                    }
                }
                (Kind::Pair, _) => {
                    let word = read(at, 8);
                    if word == 0 {
                        out.push_str("null");
                        return;
                    }
                    let held = TypeId::from_raw(word as u32);
                    match self.table.pair_payload(held) {
                        PairPayload::Boxed => self.write(out, held, (read(at.add(8), 8) as usize + BOX_VALUE_OFFSET as usize) as *const u8, top),
                        _ => self.write(out, held, at.add(8), top),
                    }
                }
                _ => self.object(out, ty, read(at, 8) as usize, top),
            }
        }
    }

    unsafe fn scalar(&self, out: &mut String, scalar: Scalar, shown: &Shown, at: *const u8) {
        // SAFETY: the caller's promise.
        let bits = unsafe { read(at, scalar.size()) };
        match (scalar, shown) {
            (_, Shown::Bool) => out.push_str(if bits != 0 { "true" } else { "false" }),
            (Scalar::F32, _) => out.push_str(&float_text(f64::from(f32::from_bits(bits as u32)), true)),
            (Scalar::F64, _) => out.push_str(&float_text(f64::from_bits(bits), false)),
            (_, Shown::Unsigned) => out.push_str(&bits.to_string()),
            (Scalar::I8, _) => out.push_str(&(bits as i8).to_string()),
            (Scalar::I16, _) => out.push_str(&(bits as i16).to_string()),
            (Scalar::I32, _) => out.push_str(&(bits as i32).to_string()),
            _ => out.push_str(&(bits as i64).to_string()),
        }
    }

    unsafe fn record(&self, out: &mut String, name: &str, names: &[String], fields: impl Iterator<Item = (TypeId, *const u8)>) {
        out.push_str(name);
        out.push('(');
        // SAFETY: the caller's promise.
        unsafe { self.fields(out, names, fields) };
        out.push(')');
    }

    /// `a, b` or `name: a, name: b` for the fields, each with its name when it has one.
    unsafe fn fields(&self, out: &mut String, names: &[String], fields: impl Iterator<Item = (TypeId, *const u8)>) {
        for (index, (ty, at)) in fields.enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            if let Some(name) = names.get(index).filter(|name| !name.is_empty()) {
                out.push_str(name);
                out.push_str(": ");
            }
            // SAFETY: the caller's promise.
            unsafe { self.write(out, ty, at, false) };
        }
    }

    unsafe fn elements(&self, out: &mut String, element: TypeId, items: impl Iterator<Item = *const u8>) {
        out.push('[');
        for (index, at) in items.enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            // SAFETY: the caller's promise.
            unsafe { self.write(out, element, at, false) };
        }
        out.push(']');
    }

    /// An object of the type, or `null` for no object.
    unsafe fn object(&self, out: &mut String, ty: TypeId, object: usize, top: bool) {
        if object == 0 {
            out.push_str("null");
            return;
        }
        let info = self.table.get(ty);
        // SAFETY: the caller's promise: the object is live and of the type.
        unsafe {
            match (&info.kind, &self.meta.shown[ty.index()]) {
                (Kind::String, _) => {
                    let value = text(object);
                    if top {
                        out.push_str(value);
                    } else {
                        out.push('"');
                        out.push_str(&value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"));
                        out.push('"');
                    }
                }
                (Kind::Array(element), _) => {
                    let (stride, length) = (self.stride(*element), *((object + LENGTH_OFFSET as usize) as *const u64) as usize);
                    self.elements(out, *element, (0..length).map(|index| (object + ELEMENTS_OFFSET as usize + index * stride) as *const u8));
                }
                (Kind::List(element), _) => {
                    let buffer = *((object + LIST_BUFFER_OFFSET as usize) as *const u64) as usize;
                    let (stride, length) = (self.stride(*element), *((object + LENGTH_OFFSET as usize) as *const u64) as usize);
                    self.elements(out, *element, (0..length).map(|index| (buffer + ELEMENTS_OFFSET as usize + index * stride) as *const u8));
                }
                (Kind::Class(_), Shown::Map) => self.map(out, object, true),
                (Kind::Class(_), Shown::Set) => {
                    let members = *((object + info.object.as_ref().expect("a class has fields").fields[0].offset as usize) as *const u64) as usize;
                    self.map(out, members, false);
                }
                (Kind::Class(_), Shown::Record(name, names)) => {
                    let fields = info.object.as_ref().expect("a class has fields").fields.iter().map(|field| (field.ty, (object + field.offset as usize) as *const u8));
                    self.record(out, name, names, fields);
                }
                (Kind::Box(inner), _) => self.write(out, *inner, (object + BOX_VALUE_OFFSET as usize) as *const u8, top),
                (Kind::Closure(_), _) => out.push_str("<function>"),
                (Kind::Handle(..), _) => out.push_str("<handle>"),
                (Kind::Class(_), _) => {
                    let fields = info.object.as_ref().expect("a class has fields").fields.iter().map(|field| (field.ty, (object + field.offset as usize) as *const u8));
                    out.push('(');
                    self.fields(out, &[], fields);
                    out.push(')');
                }
                _ => out.push_str("null"),
            }
        }
    }

    fn stride(&self, ty: TypeId) -> usize {
        let layout = self.table.layout(ty);
        mote_abi::align_up(layout.size, layout.align) as usize
    }

    /// `{key: value, ...}` for a map, or `{key, ...}` for the keys of a map that is a set, in the order they were set.
    unsafe fn map(&self, out: &mut String, object: usize, with_values: bool) {
        let info = self.table.get(self.header_type(object));
        let fields = &info.object.as_ref().expect("a map is a class").fields;
        // SAFETY: the caller's promise: a map holds `held_keys`, `held_values` and `hashes` lists, in that order.
        unsafe {
            let list = |index: usize| *((object + fields[index].offset as usize) as *const u64) as usize;
            let (keys, values, hashes) = (list(0), list(1), list(2));
            let Kind::List(key) = self.table.get(fields[0].ty).kind else { return };
            let Kind::List(value) = self.table.get(fields[1].ty).kind else { return };
            let count = *((keys + LENGTH_OFFSET as usize) as *const u64) as usize;
            let (keys_at, values_at, hashes_at) = (self.buffer(keys), self.buffer(values), self.buffer(hashes));
            out.push('{');
            let mut first = true;
            for index in 0..count {
                if *((hashes_at + index * 8) as *const i64) == -1 {
                    continue;
                }
                if !first {
                    out.push_str(", ");
                }
                first = false;
                self.write(out, key, (keys_at + index * self.stride(key)) as *const u8, false);
                if with_values {
                    out.push_str(": ");
                    self.write(out, value, (values_at + index * self.stride(value)) as *const u8, false);
                }
            }
            out.push('}');
        }
    }

    /// The address of the first element of a list.
    unsafe fn buffer(&self, list: usize) -> usize {
        // SAFETY: the caller's promise.
        unsafe { *((list + LIST_BUFFER_OFFSET as usize) as *const u64) as usize + ELEMENTS_OFFSET as usize }
    }

    fn header_type(&self, object: usize) -> TypeId {
        // SAFETY: the caller's promise: the object is live.
        mote_abi::Header::from_bits(unsafe { *(object as *const u64) }).type_id()
    }
}
