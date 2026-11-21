use crate::bytes::{BytesError, Reader, Writer};

/// The first bytes of a debug table.
pub const MAGIC: &[u8; 8] = b"MOTEDBUG";

/// The version of the byte form.
pub const VERSION: u32 = 1;

/// The place of a fault site that lies in no file.
pub const NO_PLACE: u32 = u32::MAX;

/// A position in a source file; the line and the column count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: u32,
    pub line: u32,
    pub column: u32,
}

/// One function of the code table: its name, its length in bytes, and where its code came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugFunction {
    pub name: String,
    pub size: u32,
    /// The offset from the function's start where a place begins, ascending, and the place's index.
    pub rows: Vec<(u32, u32)>,
}

/// Where the code of an object file came from: the names of functions and the files and lines of their code and of their checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugTable {
    pub files: Vec<String>,
    pub places: Vec<Place>,
    /// One for each function of the code table, in its order.
    pub functions: Vec<DebugFunction>,
    /// The place of each site of the fault table, or `NO_PLACE`.
    pub faults: Vec<u32>,
}

impl DebugTable {
    /// The file, line and column of a place by its index.
    pub fn place(&self, index: u32) -> Option<(&str, u32, u32)> {
        let place = self.places.get(index as usize)?;
        Some((self.files.get(place.file as usize)?, place.line, place.column))
    }

    /// The file, line and column of the check at a site of the fault table.
    pub fn fault_place(&self, site: u32) -> Option<(&str, u32, u32)> {
        self.place(*self.faults.get(site as usize)?)
    }

    /// The magic, the version, the files, the places, the functions as a name, a size and rows, then the places of the fault sites.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.raw(MAGIC);
        writer.u32(VERSION);
        writer.count(self.files.len());
        self.files.iter().for_each(|file| writer.text(file));
        writer.count(self.places.len());
        for place in &self.places {
            writer.u32(place.file);
            writer.u32(place.line);
            writer.u32(place.column);
        }
        writer.count(self.functions.len());
        for function in &self.functions {
            writer.text(&function.name);
            writer.u32(function.size);
            writer.count(function.rows.len());
            for (offset, place) in &function.rows {
                writer.u32(*offset);
                writer.u32(*place);
            }
        }
        writer.count(self.faults.len());
        self.faults.iter().for_each(|place| writer.u32(*place));
        writer.finish()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BytesError> {
        let mut reader = Reader::new(bytes);
        reader.expect(MAGIC)?;
        match reader.u32()? {
            VERSION => {}
            other => return Err(BytesError::Version(other)),
        }
        let files = (0..reader.count(4)?).map(|_| reader.text()).collect::<Result<Vec<_>, _>>()?;
        let mut places = Vec::new();
        for _ in 0..reader.count(12)? {
            let place = Place { file: reader.u32()?, line: reader.u32()?, column: reader.u32()? };
            if place.file as usize >= files.len() {
                return Err(BytesError::Invalid("a place names a file that is not listed"));
            }
            places.push(place);
        }
        let mut functions = Vec::new();
        for _ in 0..reader.count(12)? {
            let (name, size) = (reader.text()?, reader.u32()?);
            let mut rows: Vec<(u32, u32)> = Vec::new();
            for _ in 0..reader.count(8)? {
                let (offset, place) = (reader.u32()?, reader.u32()?);
                if rows.last().is_some_and(|(last, _)| *last >= offset) {
                    return Err(BytesError::Invalid("the rows of a function are not in order"));
                }
                if place as usize >= places.len() {
                    return Err(BytesError::Invalid("a row names a place that is not listed"));
                }
                rows.push((offset, place));
            }
            functions.push(DebugFunction { name, size, rows });
        }
        let faults = (0..reader.count(4)?).map(|_| reader.u32()).collect::<Result<Vec<_>, _>>()?;
        if faults.iter().any(|place| *place != NO_PLACE && *place as usize >= places.len()) {
            return Err(BytesError::Invalid("a fault site names a place that is not listed"));
        }
        reader.end()?;
        Ok(Self { files, places, functions, faults })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> DebugTable {
        DebugTable {
            files: vec!["src/main.mote".into(), "std/list.mote".into()],
            places: vec![Place { file: 0, line: 3, column: 5 }, Place { file: 1, line: 40, column: 1 }],
            functions: vec![DebugFunction { name: "main".into(), size: 96, rows: vec![(0, 0), (24, 1)] }, DebugFunction { name: "List.push".into(), size: 8, rows: vec![] }],
            faults: vec![NO_PLACE, 1],
        }
    }

    #[test]
    fn debug_a_table_reads_back_equal_and_gives_one_byte_string() {
        let bytes = table().to_bytes();
        let read = DebugTable::from_bytes(&bytes).unwrap();
        assert_eq!(read, table());
        assert_eq!(read.to_bytes(), bytes);
        assert_eq!(&bytes[..8], MAGIC);
    }

    #[test]
    fn debug_a_place_is_found_by_its_index_and_a_fault_site() {
        let table = table();
        assert_eq!(table.place(0), Some(("src/main.mote", 3, 5)));
        assert_eq!(table.fault_place(1), Some(("std/list.mote", 40, 1)));
        assert_eq!(table.fault_place(0), None);
        assert_eq!(table.fault_place(9), None);
    }

    #[test]
    fn debug_nothing_is_the_header_and_four_zero_counts() {
        assert_eq!(DebugTable::default().to_bytes().len(), 8 + 4 + 4 * 4);
        assert_eq!(DebugTable::from_bytes(&DebugTable::default().to_bytes()), Ok(DebugTable::default()));
    }

    #[test]
    fn debug_a_bad_table_is_refused() {
        let bytes = table().to_bytes();
        assert_eq!(DebugTable::from_bytes(b"MOTECODE\x01\0\0\0"), Err(BytesError::Magic));
        let mut other = bytes.clone();
        other[8] = 2;
        assert_eq!(DebugTable::from_bytes(&other), Err(BytesError::Version(2)));
        let mut more = bytes.clone();
        more.push(0);
        assert_eq!(DebugTable::from_bytes(&more), Err(BytesError::Trailing));
        for cut in 0..bytes.len() {
            assert!(DebugTable::from_bytes(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut unordered = table();
        unordered.functions[0].rows = vec![(9, 0), (9, 1)];
        assert_eq!(DebugTable::from_bytes(&unordered.to_bytes()), Err(BytesError::Invalid("the rows of a function are not in order")));
        let mut lost = table();
        lost.places[0].file = 5;
        assert_eq!(DebugTable::from_bytes(&lost.to_bytes()), Err(BytesError::Invalid("a place names a file that is not listed")));
        let mut dangling = table();
        dangling.faults = vec![7];
        assert_eq!(DebugTable::from_bytes(&dangling.to_bytes()), Err(BytesError::Invalid("a fault site names a place that is not listed")));
    }
}
