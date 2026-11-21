//! Where `mote_tables`, the directory of an object file, keeps what the runtime of an executable reads.

/// The words of the directory.
pub const DIRECTORY_WORDS: usize = 18;

/// The index of the word that holds the address of the type table; its byte length follows.
pub const DIRECTORY_TYPES: usize = 0;
/// The address of the fault table, then its byte length.
pub const DIRECTORY_FAULTS: usize = 2;
/// The address of the code table, then its byte length.
pub const DIRECTORY_CODE: usize = 4;
/// The address of the array of address words of the code table, then its number of words.
pub const DIRECTORY_WORDS_OF_CODE: usize = 6;
/// The address of the meta table, then its byte length.
pub const DIRECTORY_META: usize = 8;
/// The address of the link table, then its byte length.
pub const DIRECTORY_LINK: usize = 10;
/// The address of the two words of the run state, or 0.
pub const DIRECTORY_FAILURE: usize = 12;
/// The address of the native slots, a word for each linked function, or 0.
pub const DIRECTORY_NATIVES: usize = 13;
/// The address of the array of `mote_init_<name>` functions of the statically linked libraries, then its number of words; a library without one has 0.
pub const DIRECTORY_INITS: usize = 14;
/// The address of the debug table, then its byte length.
pub const DIRECTORY_DEBUG: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_the_tables_follow_each_other_as_an_address_and_a_length_and_the_cells_come_last() {
        let tables = [DIRECTORY_TYPES, DIRECTORY_FAULTS, DIRECTORY_CODE, DIRECTORY_WORDS_OF_CODE, DIRECTORY_META, DIRECTORY_LINK, DIRECTORY_INITS, DIRECTORY_DEBUG];
        assert_eq!(tables, [0, 2, 4, 6, 8, 10, 14, 16]);
        assert_eq!([DIRECTORY_FAILURE, DIRECTORY_NATIVES], [12, 13]);
        assert_eq!(DIRECTORY_WORDS, 18);
    }
}
