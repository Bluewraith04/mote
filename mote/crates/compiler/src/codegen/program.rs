//! `CompiledProgram` and its `.mbc` byte format.

use super::*;

pub use isa::code::SourceFile;

pub(crate) fn collect_sources(code_objects: &mut [CodeObject]) -> Vec<SourceFile> {
    let mut renumber: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut sources = Vec::new();
    for span in code_objects.iter_mut().flat_map(|c| c.spans.iter_mut()) {
        let id = *renumber.entry(span.source).or_insert_with(|| match crate::span::source_info(span.source) {
            Some((path, text)) => {
                let id = sources.len() as u32 + 1;
                sources.push(SourceFile { id, path, text });
                id
            }
            None => 0,
        });
        span.source = id;
    }
    sources
}

#[derive(Clone, Debug)]
/// The compiled output: code objects, type descriptors, string table and native table.
pub struct CompiledProgram {
    pub code_objects: Vec<CodeObject>,
    pub type_descriptors: Vec<TypeDescriptor>,
    /// Number of module-level global slots the VM allocates.
    pub global_count: u32,
    /// Builtin natives this program calls, by registry name; `CALLNATIVEW` operands index it.
    pub native_table: Vec<String>,
    /// The texts the code objects' span tables index; empty in a release build.
    pub sources: Vec<SourceFile>,
    /// Where each struct literal lives, when compiled with `with_memory_report`; never serialized.
    pub memory_sites: Vec<crate::regions::MemorySite>,
}

impl CompiledProgram {
    pub const MAGIC: &'static [u8; 6] = b"MOTE\x01\x11";

    /// Serializes the compiled program into a compact binary format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(Self::MAGIC);
        buf.extend_from_slice(&self.global_count.to_le_bytes());

        buf.extend_from_slice(&(self.type_descriptors.len() as u32).to_le_bytes());
        for td in &self.type_descriptors {
            buf.extend_from_slice(&td.id.to_le_bytes());
            buf.extend_from_slice(&td.pointer_bitmap.to_le_bytes());
            buf.push(if td.is_value_type { 1 } else { 0 });
            buf.push(if td.is_trivial { 1 } else { 0 });
            buf.push(if td.by_content { 1 } else { 0 });
            buf.extend_from_slice(&td.slots.to_le_bytes());
            buf.extend_from_slice(&(td.fields.len() as u32).to_le_bytes());
            for f in &td.fields {
                buf.push(if f.is_pointer { 1 } else { 0 });
                buf.push(if f.is_pub { 1 } else { 0 });
                buf.extend_from_slice(&f.slot.to_le_bytes());
                buf.extend_from_slice(&f.inline.as_ref().map_or(u32::MAX, |d| d.id as u32).to_le_bytes());
                if let Some(ref name) = f.name {
                    let name_bytes = name.as_bytes();
                    buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                    buf.extend_from_slice(name_bytes);
                } else {
                    buf.extend_from_slice(&0u16.to_le_bytes());
                }
            }
            match &td.name {
                Some(name) => {
                    let name_bytes = name.as_bytes();
                    buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
                    buf.extend_from_slice(name_bytes);
                }
                None => buf.extend_from_slice(&0u16.to_le_bytes()),
            }
            match &td.instance {
                Some(term) => {
                    buf.push(1);
                    term.encode(&mut buf);
                }
                None => buf.push(0),
            }
            match &td.module {
                Some(module) => {
                    buf.push(1);
                    buf.extend_from_slice(&(module.len() as u16).to_le_bytes());
                    buf.extend_from_slice(module.as_bytes());
                }
                None => buf.push(0),
            }
        }

        buf.extend_from_slice(&(self.code_objects.len() as u32).to_le_bytes());
        for co in &self.code_objects {
            buf.extend_from_slice(&co.register_count.to_le_bytes());
            buf.push(co.param_count);

            buf.extend_from_slice(&(co.instructions.len() as u32).to_le_bytes());
            for &inst in &co.instructions {
                buf.extend_from_slice(&inst.to_le_bytes());
            }

            buf.extend_from_slice(&(co.constants.len() as u32).to_le_bytes());
            for &val in &co.constants {
                let (tag, payload) = val.to_wire();
                buf.push(tag);
                buf.extend_from_slice(&payload);
            }

            buf.extend_from_slice(&(co.string_table.len() as u32).to_le_bytes());
            for s in &co.string_table {
                let sb = s.as_bytes();
                buf.extend_from_slice(&(sb.len() as u32).to_le_bytes());
                buf.extend_from_slice(sb);
            }

            buf.extend_from_slice(&(co.spans.len() as u32).to_le_bytes());
            for s in &co.spans {
                for field in [s.start_pc, s.end_pc, s.source, s.offset, s.len, s.line, s.col] {
                    buf.extend_from_slice(&field.to_le_bytes());
                }
            }
        }

        buf.extend_from_slice(&(self.native_table.len() as u32).to_le_bytes());
        for name in &self.native_table {
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(name.as_bytes());
        }

        buf.extend_from_slice(&(self.sources.len() as u32).to_le_bytes());
        for src in &self.sources {
            buf.extend_from_slice(&src.id.to_le_bytes());
            for text in [&src.path, &src.text] {
                buf.extend_from_slice(&(text.len() as u32).to_le_bytes());
                buf.extend_from_slice(text.as_bytes());
            }
        }

        buf
    }

    /// Deserializes a binary buffer back into a CompiledProgram.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut cursor = 0;
        if bytes.len() < 6 || &bytes[0..6] != Self::MAGIC {
            return Err("Invalid Mote bytecode header magic".to_string());
        }
        cursor += 6;

        let read_u16 = |cur: &mut usize| -> Result<u16, String> {
            if *cur + 2 > bytes.len() {
                return Err("Unexpected EOF reading u16".into());
            }
            let val = u16::from_le_bytes([bytes[*cur], bytes[*cur + 1]]);
            *cur += 2;
            Ok(val)
        };

        let read_u32 = |cur: &mut usize| -> Result<u32, String> {
            if *cur + 4 > bytes.len() {
                return Err("Unexpected EOF reading u32".into());
            }
            let val = u32::from_le_bytes([bytes[*cur], bytes[*cur + 1], bytes[*cur + 2], bytes[*cur + 3]]);
            *cur += 4;
            Ok(val)
        };

        let read_u64 = |cur: &mut usize| -> Result<u64, String> {
            if *cur + 8 > bytes.len() {
                return Err("Unexpected EOF reading u64".into());
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[*cur..*cur + 8]);
            *cur += 8;
            Ok(u64::from_le_bytes(b))
        };

        let read_u8 = |cur: &mut usize| -> Result<u8, String> {
            if *cur + 1 > bytes.len() {
                return Err("Unexpected EOF reading u8".into());
            }
            let val = bytes[*cur];
            *cur += 1;
            Ok(val)
        };

        let global_count = read_u32(&mut cursor)?;

        let td_count = read_u32(&mut cursor)? as usize;
        let mut type_descriptors = Vec::with_capacity(td_count);
        let mut inline_ids: Vec<Vec<Option<u32>>> = Vec::with_capacity(td_count);
        for _ in 0..td_count {
            let id = read_u64(&mut cursor)?;
            let pointer_bitmap = read_u64(&mut cursor)?;
            let is_value_type = read_u8(&mut cursor)? != 0;
            let is_trivial = read_u8(&mut cursor)? != 0;
            let by_content = read_u8(&mut cursor)? != 0;
            let slots = read_u32(&mut cursor)?;
            let fields_count = read_u32(&mut cursor)? as usize;
            let mut fields = Vec::with_capacity(fields_count);
            let mut field_inlines = Vec::with_capacity(fields_count);
            for _ in 0..fields_count {
                let is_pointer = read_u8(&mut cursor)? != 0;
                let is_pub = read_u8(&mut cursor)? != 0;
                let slot = read_u32(&mut cursor)?;
                field_inlines.push(Some(read_u32(&mut cursor)?).filter(|&n| n != u32::MAX));
                let name_len = read_u16(&mut cursor)? as usize;
                let name = if name_len > 0 {
                    if cursor + name_len > bytes.len() {
                        return Err("Unexpected EOF reading field name".into());
                    }
                    let s = std::str::from_utf8(&bytes[cursor..cursor + name_len])
                        .map_err(|e| format!("Invalid UTF-8 field name: {}", e))?;
                    cursor += name_len;
                    Some(s.to_string())
                } else {
                    None
                };
                fields.push(isa::value::FieldDescriptor { name, is_pointer, is_pub, slot, inline: None });
            }
            let name_len = read_u16(&mut cursor)? as usize;
            let name = if name_len > 0 {
                if cursor + name_len > bytes.len() {
                    return Err("Unexpected EOF reading type name".into());
                }
                let s = std::str::from_utf8(&bytes[cursor..cursor + name_len])
                    .map_err(|e| format!("Invalid UTF-8 type name: {}", e))?;
                cursor += name_len;
                Some(s.to_string())
            } else {
                None
            };
            let instance = match read_u8(&mut cursor)? {
                0 => None,
                _ => Some(isa::type_term::TypeTerm::decode(bytes, &mut cursor)?),
            };
            let module = match read_u8(&mut cursor)? {
                0 => None,
                _ => {
                    let len = read_u16(&mut cursor)? as usize;
                    if cursor + len > bytes.len() {
                        return Err("Unexpected EOF reading module name".into());
                    }
                    let s = std::str::from_utf8(&bytes[cursor..cursor + len]).map_err(|e| format!("Invalid UTF-8 module name: {}", e))?;
                    cursor += len;
                    Some(s.to_string())
                }
            };
            type_descriptors.push(TypeDescriptor {
                id,
                fields,
                slots,
                pointer_bitmap,
                is_value_type,
                is_trivial,
                by_content,
                name,
                layout: isa::value::SlotLayout::Fixed,
                instance,
                module,
            });
            inline_ids.push(field_inlines);
        }
        if inline_ids.iter().flatten().flatten().any(|&n| n as usize >= td_count) {
            return Err("Embedded struct index out of range".into());
        }
        isa::value::link_inline_fields(&mut type_descriptors, &inline_ids);

        let co_count = read_u32(&mut cursor)? as usize;
        let mut code_objects = Vec::with_capacity(co_count);
        for _ in 0..co_count {
            let register_count = read_u16(&mut cursor)?;
            let param_count = read_u8(&mut cursor)?;

            let inst_count = read_u32(&mut cursor)? as usize;
            let mut instructions = Vec::with_capacity(inst_count);
            for _ in 0..inst_count {
                instructions.push(read_u32(&mut cursor)?);
            }

            let const_count = read_u32(&mut cursor)? as usize;
            let mut constants = Vec::with_capacity(const_count);
            for _ in 0..const_count {
                let tag_byte = read_u8(&mut cursor)?;
                if cursor + 8 > bytes.len() {
                    return Err("Unexpected EOF reading constant payload".into());
                }
                let mut raw_bytes = [0u8; 8];
                raw_bytes.copy_from_slice(&bytes[cursor..cursor + 8]);
                cursor += 8;

                constants.push(Value::from_wire(tag_byte, raw_bytes)?);
            }

            let str_count = read_u32(&mut cursor)? as usize;
            let mut string_table = Vec::with_capacity(str_count);
            for _ in 0..str_count {
                let slen = read_u32(&mut cursor)? as usize;
                if cursor + slen > bytes.len() {
                    return Err("Unexpected EOF reading string-table entry".into());
                }
                let s = std::str::from_utf8(&bytes[cursor..cursor + slen])
                    .map_err(|e| format!("Invalid UTF-8 string-table entry: {}", e))?
                    .to_string();
                cursor += slen;
                string_table.push(s);
            }

            let span_count = read_u32(&mut cursor)? as usize;
            let mut spans = Vec::with_capacity(span_count);
            for _ in 0..span_count {
                spans.push(isa::code::SourceSpan {
                    start_pc: read_u32(&mut cursor)?,
                    end_pc: read_u32(&mut cursor)?,
                    source: read_u32(&mut cursor)?,
                    offset: read_u32(&mut cursor)?,
                    len: read_u32(&mut cursor)?,
                    line: read_u32(&mut cursor)?,
                    col: read_u32(&mut cursor)?,
                });
            }

            code_objects.push(
                CodeObject::new(instructions, constants, register_count, param_count)
                    .with_string_table(string_table)
                    .with_spans(spans),
            );
        }

        let table_len = read_u32(&mut cursor)? as usize;
        let mut native_table = Vec::with_capacity(table_len);
        for _ in 0..table_len {
            let n = read_u32(&mut cursor)? as usize;
            if cursor + n > bytes.len() {
                return Err("Unexpected EOF reading native table name".into());
            }
            let name = std::str::from_utf8(&bytes[cursor..cursor + n])
                .map_err(|e| format!("Invalid UTF-8 native table name: {}", e))?;
            native_table.push(name.to_string());
            cursor += n;
        }

        let source_count = read_u32(&mut cursor)? as usize;
        let mut sources = Vec::with_capacity(source_count);
        for _ in 0..source_count {
            let id = read_u32(&mut cursor)?;
            let mut texts = Vec::with_capacity(2);
            for _ in 0..2 {
                let n = read_u32(&mut cursor)? as usize;
                if cursor + n > bytes.len() {
                    return Err("Unexpected EOF reading a source file".into());
                }
                let text = std::str::from_utf8(&bytes[cursor..cursor + n])
                    .map_err(|e| format!("Invalid UTF-8 source file: {}", e))?;
                texts.push(text.to_string());
                cursor += n;
            }
            let text = texts.pop().unwrap_or_default();
            let path = texts.pop().unwrap_or_default();
            sources.push(SourceFile { id, path, text });
        }

        Ok(CompiledProgram {
            code_objects,
            type_descriptors,
            global_count,
            native_table,
            sources,
            memory_sites: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_table_round_trips_through_to_bytes_and_from_bytes() {
        let program = CompiledProgram {
            code_objects: vec![CodeObject::new(vec![], vec![], 0, 0)],
            type_descriptors: vec![],
            global_count: 0,
            native_table: vec!["print".to_string(), "fs_read".to_string()],
            sources: vec![],
            memory_sites: vec![],
        };
        let bytes = program.to_bytes();
        assert_eq!(&bytes[0..6], CompiledProgram::MAGIC);
        assert_eq!(CompiledProgram::from_bytes(&bytes).unwrap().native_table, program.native_table);
    }

    #[test]
    fn a_type_keeps_its_size_and_field_slots_through_the_bytes() {
        let mut td = TypeDescriptor::new_value_type(7, vec![Some("a".into()), Some("b".into())], true);
        td.slots = 5;
        td.fields[1].slot = 3;
        let program = CompiledProgram {
            code_objects: vec![CodeObject::new(vec![], vec![], 0, 0)],
            type_descriptors: vec![td],
            global_count: 0,
            native_table: vec![],
            sources: vec![],
            memory_sites: vec![],
        };
        let back = CompiledProgram::from_bytes(&program.to_bytes()).unwrap();
        assert_eq!(back.type_descriptors[0].slots, 5);
        assert_eq!(back.type_descriptors[0].fields[1].slot, 3);
    }
}

