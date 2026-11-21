use mote_base::Span;

use crate::bytes::{BytesError, Reader, Writer};
use crate::scalar::Scalar;

/// A C type a native function's parameter or result may have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CType {
    /// A result only.
    Void,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    /// `const char *`, a NUL-terminated copy valid for the call; a parameter only.
    StringIn,
    /// `char *`, copied into a string and never freed; a result only.
    StringOut,
    /// A pointer to the elements, then the length as `int64_t`; a parameter only.
    Array,
    /// A `Pointer<T>`; a NULL result is a fault.
    Pointer,
    /// A `Pointer<T>?`; `None` is NULL.
    NullablePointer,
    /// A `PortEnd`, an opaque `MotePort *` the callee owns one reference to; a parameter only.
    PortEnd,
}

impl CType {
    pub const ALL: [CType; 17] = [
        CType::Void,
        CType::I8,
        CType::I16,
        CType::I32,
        CType::I64,
        CType::U8,
        CType::U16,
        CType::U32,
        CType::U64,
        CType::F32,
        CType::F64,
        CType::StringIn,
        CType::StringOut,
        CType::Array,
        CType::Pointer,
        CType::NullablePointer,
        CType::PortEnd,
    ];

    /// The machine values it travels as.
    pub fn scalars(self) -> &'static [Scalar] {
        match self {
            CType::Void => &[],
            CType::I8 | CType::U8 => &[Scalar::I8],
            CType::I16 | CType::U16 => &[Scalar::I16],
            CType::I32 | CType::U32 => &[Scalar::I32],
            CType::I64 | CType::U64 | CType::StringIn | CType::StringOut | CType::Pointer | CType::NullablePointer | CType::PortEnd => &[Scalar::I64],
            CType::F32 => &[Scalar::F32],
            CType::F64 => &[Scalar::F64],
            CType::Array => &[Scalar::I64, Scalar::I64],
        }
    }

    /// Whether the value widens by sign when it is smaller than a register.
    pub fn signed(self) -> bool {
        matches!(self, CType::I8 | CType::I16 | CType::I32 | CType::I64)
    }

    pub fn is_parameter(self) -> bool {
        !matches!(self, CType::Void | CType::StringOut)
    }

    pub fn is_result(self) -> bool {
        !matches!(self, CType::StringIn | CType::Array | CType::PortEnd)
    }

    /// Whether a `var` parameter of this type may be passed as a pointer to it.
    pub fn can_be_by_ref(self) -> bool {
        !matches!(self, CType::Void | CType::StringIn | CType::StringOut | CType::Array | CType::PortEnd)
    }
}

/// A parameter of a native function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CParam {
    pub ty: CType,
    /// A `var` parameter: a pointer to a temporary, copied back after the call.
    pub by_ref: bool,
}

impl CParam {
    pub fn value(ty: CType) -> Self {
        Self { ty, by_ref: false }
    }

    pub fn by_ref(ty: CType) -> Self {
        Self { ty, by_ref: true }
    }

    pub fn scalars(self) -> &'static [Scalar] {
        if self.by_ref { &[Scalar::I64] } else { self.ty.scalars() }
    }
}

/// The C signature of a native function.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NativeSig {
    pub params: Vec<CParam>,
    pub ret: CType,
    /// The call runs on a helper thread.
    pub blocking: bool,
}

impl NativeSig {
    /// Whether each type is allowed where it stands.
    pub fn is_valid(&self) -> bool {
        self.params.iter().all(|param| param.ty.is_parameter() && (!param.by_ref || param.ty.can_be_by_ref())) && self.ret.is_result()
    }

    /// The machine parameters, an array taking two and a by-reference parameter one pointer.
    pub fn param_scalars(&self) -> Vec<Scalar> {
        self.params.iter().flat_map(|param| param.scalars().iter().copied()).collect()
    }

    pub fn result_scalars(&self) -> &'static [Scalar] {
        self.ret.scalars()
    }
}

/// A native function the program loads before `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedFn {
    /// An index into the table's libraries.
    pub library: u32,
    pub symbol: String,
    pub sig: NativeSig,
    /// Where the function is declared, for the error when it cannot be loaded.
    pub span: Span,
}

/// The libraries and symbols a program needs at start-up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkTable {
    pub libraries: Vec<String>,
    pub functions: Vec<LinkedFn>,
}

impl LinkTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a function and returns its position; a library is listed once.
    pub fn add(&mut self, library: &str, symbol: &str, sig: NativeSig, span: Span) -> u32 {
        let library = match self.libraries.iter().position(|known| known == library) {
            Some(index) => index,
            None => {
                self.libraries.push(library.to_string());
                self.libraries.len() - 1
            }
        };
        self.functions.push(LinkedFn { library: library as u32, symbol: symbol.to_string(), sig, span });
        (self.functions.len() - 1) as u32
    }
}

/// The first bytes of a link table.
pub const LINK_MAGIC: &[u8; 8] = b"MOTELINK";

/// The version of the byte form.
pub const LINK_VERSION: u32 = 1;

/// A link table with the file the loader opens for each of its libraries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkImage {
    pub table: LinkTable,
    /// For each library in order, the file to open; empty when the loader chooses.
    pub files: Vec<String>,
}

impl CType {
    fn tag(self) -> u8 {
        CType::ALL.iter().position(|known| *known == self).expect("every type is listed") as u8
    }
}

impl LinkImage {
    /// The magic, the version, the library names, the files, then each function as its library, symbol, parameters, result, `blocking` and span.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.raw(LINK_MAGIC);
        writer.u32(LINK_VERSION);
        for texts in [&self.table.libraries, &self.files] {
            writer.count(texts.len());
            texts.iter().for_each(|text| writer.text(text));
        }
        writer.count(self.table.functions.len());
        for function in &self.table.functions {
            writer.u32(function.library);
            writer.text(&function.symbol);
            writer.count(function.sig.params.len());
            for param in &function.sig.params {
                writer.u8(param.ty.tag());
                writer.u8(u8::from(param.by_ref));
            }
            writer.u8(function.sig.ret.tag());
            writer.u8(u8::from(function.sig.blocking));
            writer.u32(function.span.start);
            writer.u32(function.span.end);
        }
        writer.finish()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BytesError> {
        let mut reader = Reader::new(bytes);
        reader.expect(LINK_MAGIC)?;
        match reader.u32()? {
            LINK_VERSION => {}
            other => return Err(BytesError::Version(other)),
        }
        let texts = |reader: &mut Reader| (0..reader.count(4)?).map(|_| reader.text()).collect::<Result<Vec<_>, _>>();
        let libraries = texts(&mut reader)?;
        let files = texts(&mut reader)?;
        if !files.is_empty() && files.len() != libraries.len() {
            return Err(BytesError::Invalid("a library has no file"));
        }
        let ctype = |reader: &mut Reader| {
            let tag = reader.u8()?;
            CType::ALL.get(usize::from(tag)).copied().ok_or(BytesError::Tag(tag))
        };
        let mut functions = Vec::new();
        for _ in 0..reader.count(22)? {
            let library = reader.u32()?;
            if library as usize >= libraries.len() {
                return Err(BytesError::Invalid("a function names a library that is missing"));
            }
            let symbol = reader.text()?;
            let mut params = Vec::new();
            for _ in 0..reader.count(2)? {
                params.push(CParam { ty: ctype(&mut reader)?, by_ref: reader.u8()? != 0 });
            }
            let ret = ctype(&mut reader)?;
            let blocking = reader.u8()? != 0;
            let span = Span::new(reader.u32()?, reader.u32()?);
            let sig = NativeSig { params, ret, blocking };
            if !sig.is_valid() {
                return Err(BytesError::Invalid("a signature has a type where it is not allowed"));
            }
            functions.push(LinkedFn { library, symbol, sig, span });
        }
        reader.end()?;
        Ok(Self { table: LinkTable { libraries, functions }, files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> LinkImage {
        let mut table = LinkTable::new();
        table.add("sqlite", "open", NativeSig { params: vec![CParam::value(CType::StringIn), CParam::by_ref(CType::I32)], ret: CType::Pointer, blocking: true }, Span::new(10, 24));
        table.add("sqlite", "close", NativeSig { params: vec![CParam::value(CType::Pointer)], ret: CType::Void, blocking: false }, Span::new(30, 42));
        table.add("wire", "send", NativeSig { params: vec![CParam::value(CType::Array), CParam::value(CType::PortEnd)], ret: CType::NullablePointer, blocking: false }, Span::new(50, 60));
        LinkImage { table, files: vec!["/opt/lib/libsqlite.so".into(), "libwire.so".into()] }
    }

    #[test]
    fn link_a_table_is_the_magic_the_version_the_libraries_the_files_then_the_functions() {
        let mut table = LinkTable::new();
        table.add("a", "f", NativeSig { params: vec![CParam::value(CType::I32)], ret: CType::Void, blocking: true }, Span::new(1, 2));
        let written = LinkImage { table, files: vec![] }.to_bytes();
        let mut expected = b"MOTELINK".to_vec();
        expected.extend([1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'a', 0, 0, 0, 0, 1, 0, 0, 0]);
        expected.extend([0, 0, 0, 0, 1, 0, 0, 0, b'f', 1, 0, 0, 0, 3, 0, 0, 1, 1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(written, expected);
    }

    #[test]
    fn link_a_table_reads_back_equal_and_gives_one_byte_string() {
        let bytes = image().to_bytes();
        let read = LinkImage::from_bytes(&bytes).unwrap();
        assert_eq!(read, image());
        assert_eq!(read.to_bytes(), bytes);
        assert_eq!(LinkImage::from_bytes(&LinkImage::default().to_bytes()), Ok(LinkImage::default()));
    }

    #[test]
    fn link_a_bad_table_is_refused() {
        let bytes = image().to_bytes();
        assert_eq!(LinkImage::from_bytes(b"MOTEMETA\x01\0\0\0"), Err(BytesError::Magic));
        let mut other = bytes.clone();
        other[8] = 2;
        assert_eq!(LinkImage::from_bytes(&other), Err(BytesError::Version(2)));
        let mut more = bytes.clone();
        more.push(0);
        assert_eq!(LinkImage::from_bytes(&more), Err(BytesError::Trailing));
        for cut in 0..bytes.len() {
            assert!(LinkImage::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut short = image();
        short.files.pop();
        assert_eq!(LinkImage::from_bytes(&short.to_bytes()), Err(BytesError::Invalid("a library has no file")));
        let mut lost = image();
        lost.table.functions[2].library = 2;
        assert_eq!(LinkImage::from_bytes(&lost.to_bytes()), Err(BytesError::Invalid("a function names a library that is missing")));
        let mut wrong = image();
        wrong.table.functions[0].sig.ret = CType::StringIn;
        assert_eq!(LinkImage::from_bytes(&wrong.to_bytes()), Err(BytesError::Invalid("a signature has a type where it is not allowed")));
    }

    #[test]
    fn all_lists_each_type_once_and_a_new_type_must_be_added_to_it() {
        fn position(ty: CType) -> usize {
            match ty {
                CType::Void => 0,
                CType::I8 => 1,
                CType::I16 => 2,
                CType::I32 => 3,
                CType::I64 => 4,
                CType::U8 => 5,
                CType::U16 => 6,
                CType::U32 => 7,
                CType::U64 => 8,
                CType::F32 => 9,
                CType::F64 => 10,
                CType::StringIn => 11,
                CType::StringOut => 12,
                CType::Array => 13,
                CType::Pointer => 14,
                CType::NullablePointer => 15,
                CType::PortEnd => 16,
            }
        }
        for (index, ty) in CType::ALL.iter().enumerate() {
            assert_eq!(position(*ty), index);
        }
    }

    #[test]
    fn each_type_lowers_to_its_machine_values() {
        assert_eq!(CType::Void.scalars(), []);
        assert_eq!(CType::U8.scalars(), [Scalar::I8]);
        assert_eq!(CType::I32.scalars(), [Scalar::I32]);
        assert_eq!(CType::F32.scalars(), [Scalar::F32]);
        assert_eq!(CType::StringIn.scalars(), [Scalar::I64]);
        assert_eq!(CType::Array.scalars(), [Scalar::I64, Scalar::I64]);
    }

    #[test]
    fn only_signed_integers_widen_by_sign() {
        assert!(CType::I16.signed());
        assert!(!CType::U16.signed());
        assert!(!CType::F64.signed());
    }

    #[test]
    fn some_types_are_one_direction_only() {
        assert!(!CType::Void.is_parameter());
        assert!(!CType::StringOut.is_parameter());
        assert!(!CType::StringIn.is_result());
        assert!(!CType::Array.is_result());
        assert!(CType::StringIn.is_parameter() && CType::StringOut.is_result());
        assert!(CType::I64.is_parameter() && CType::I64.is_result());
    }

    #[test]
    fn a_signature_checks_its_directions_and_flattens_arrays() {
        let sig = NativeSig { params: vec![CParam::value(CType::F64), CParam::value(CType::Array)], ret: CType::I32, blocking: false };
        assert!(sig.is_valid());
        assert_eq!(sig.param_scalars(), [Scalar::F64, Scalar::I64, Scalar::I64]);
        assert_eq!(sig.result_scalars(), [Scalar::I32]);
        assert!(!NativeSig { params: vec![CParam::value(CType::Void)], ret: CType::Void, blocking: false }.is_valid());
        assert!(!NativeSig { params: vec![], ret: CType::Array, blocking: false }.is_valid());
    }

    #[test]
    fn pointers_are_one_word_whether_or_not_they_may_be_null() {
        assert_eq!(CType::Pointer.scalars(), [Scalar::I64]);
        assert_eq!(CType::NullablePointer.scalars(), [Scalar::I64]);
        assert!(CType::Pointer.is_parameter() && CType::Pointer.is_result());
    }

    #[test]
    fn a_by_reference_parameter_is_one_pointer_and_only_for_plain_values() {
        let sig = NativeSig { params: vec![CParam::by_ref(CType::I32), CParam::by_ref(CType::NullablePointer)], ret: CType::I32, blocking: false };
        assert!(sig.is_valid());
        assert_eq!(sig.param_scalars(), [Scalar::I64, Scalar::I64]);
        for bad in [CType::StringIn, CType::Array] {
            assert!(!NativeSig { params: vec![CParam::by_ref(bad)], ret: CType::Void, blocking: false }.is_valid());
        }
    }

    #[test]
    fn a_port_end_is_a_one_word_parameter_only() {
        assert_eq!(CType::PortEnd.scalars(), [Scalar::I64]);
        assert!(CType::PortEnd.is_parameter() && !CType::PortEnd.is_result());
        assert!(NativeSig { params: vec![CParam::value(CType::PortEnd)], ret: CType::Void, blocking: false }.is_valid());
        assert!(!NativeSig { params: vec![CParam::by_ref(CType::PortEnd)], ret: CType::Void, blocking: false }.is_valid());
        assert!(!NativeSig { params: vec![], ret: CType::PortEnd, blocking: false }.is_valid());
    }

    #[test]
    fn a_library_is_listed_once() {
        let mut table = LinkTable::new();
        let sig = NativeSig { params: vec![CParam::value(CType::F64)], ret: CType::F64, blocking: false };
        let a = table.add("m", "sin", sig.clone(), Span::new(0, 3));
        let b = table.add("c", "puts", sig.clone(), Span::new(4, 8));
        let c = table.add("m", "cos", sig, Span::new(9, 12));
        assert_eq!((a, b, c), (0, 1, 2));
        assert_eq!(table.libraries, ["m", "c"]);
        assert_eq!(table.functions[2].library, 0);
        assert_eq!(table.functions[2].symbol, "cos");
    }
}
