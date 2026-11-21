use crate::bytes::{BytesError, Reader, Writer};

/// The numbers and texts `std.meta` reads, as the type table carries them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reflection {
    pub ints: Vec<i128>,
    pub texts: Vec<String>,
}

impl Reflection {
    /// The number count, the numbers as 16 bytes each, the text count, then the texts.
    pub fn write(&self, writer: &mut Writer) {
        writer.count(self.ints.len());
        for int in &self.ints {
            writer.i128(*int);
        }
        writer.count(self.texts.len());
        for text in &self.texts {
            writer.text(text);
        }
    }

    pub fn read(reader: &mut Reader) -> Result<Self, BytesError> {
        let ints = (0..reader.count(16)?).map(|_| reader.i128()).collect::<Result<Vec<_>, _>>()?;
        let texts = (0..reader.count(4)?).map(|_| reader.text()).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { ints, texts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(reflection: &Reflection) -> Vec<u8> {
        let mut writer = Writer::new();
        reflection.write(&mut writer);
        writer.finish()
    }

    #[test]
    fn reflection_reads_back_equal() {
        let reflection = Reflection { ints: vec![0, 1, -1, i128::MAX, i128::MIN, 1 << 100], texts: vec![String::new(), "Point".into(), "x: Int".into(), "é✓".into()] };
        let written = bytes(&reflection);
        let mut reader = Reader::new(&written);
        assert_eq!(Reflection::read(&mut reader), Ok(reflection));
        assert_eq!(reader.end(), Ok(()));
    }

    #[test]
    fn reflection_is_the_counts_the_numbers_then_the_texts() {
        let written = bytes(&Reflection { ints: vec![5], texts: vec!["ab".into()] });
        let mut expected = vec![1, 0, 0, 0, 5];
        expected.extend([0; 15]);
        expected.extend([1, 0, 0, 0, 2, 0, 0, 0, b'a', b'b']);
        assert_eq!(written, expected);
    }

    #[test]
    fn reflection_nothing_is_two_zero_counts() {
        assert_eq!(bytes(&Reflection::default()), [0; 8]);
    }

    #[test]
    fn reflection_a_cut_short_section_is_refused() {
        let written = bytes(&Reflection { ints: vec![1, 2], texts: vec!["abc".into()] });
        for cut in 0..written.len() {
            assert!(Reflection::read(&mut Reader::new(&written[..cut])).is_err(), "cut at {cut}");
        }
    }
}
