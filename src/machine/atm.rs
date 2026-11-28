//! `.atm` files: a program saved straight from Atom memory.
//!
//! A 22-byte header followed by the data:
//!
//! | Bytes | Meaning |
//! |-------|---------|
//! | 0-15  | name, padded with zeros |
//! | 16-17 | load address (little endian) |
//! | 18-19 | execution address |
//! | 20-21 | length of the data |

const HEADER_LEN: usize = 22;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtmFile {
    pub name: String,
    pub load_address: u16,
    pub exec_address: u16,
    pub data: Vec<u8>,
}

impl AtmFile {
    pub fn parse(file: &[u8]) -> Result<Self, String> {
        if file.len() < HEADER_LEN {
            return Err("file is too short to be an .atm file".to_string());
        }
        let word = |at: usize| u16::from_le_bytes([file[at], file[at + 1]]);
        let length = word(20) as usize;
        let data = file
            .get(HEADER_LEN..HEADER_LEN + length)
            .ok_or_else(|| format!("header says {length} bytes but the file is shorter"))?;

        let name: String = file[..16]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| {
                if (0x20..0x7F).contains(&b) {
                    b as char
                } else {
                    '?'
                }
            })
            .collect();
        Ok(AtmFile {
            name,
            load_address: word(16),
            exec_address: word(18),
            data: data.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_header_and_data() {
        let mut file = b"HELLO".to_vec();
        file.resize(16, 0);
        file.extend([0x00, 0x29, 0xB2, 0xC2, 0x03, 0x00]); // load 2900, exec C2B2, len 3
        file.extend([1, 2, 3, 99]); // trailing junk is ignored
        let atm = AtmFile::parse(&file).unwrap();
        assert_eq!(atm.name, "HELLO");
        assert_eq!((atm.load_address, atm.exec_address), (0x2900, 0xC2B2));
        assert_eq!(atm.data, vec![1, 2, 3]);
    }

    #[test]
    fn rejects_truncated_files() {
        assert!(AtmFile::parse(&[0; 10]).is_err());
        let mut file = vec![0; 22];
        file[20] = 5; // claims 5 bytes of data, has none
        assert!(AtmFile::parse(&file).is_err());
    }

    #[test]
    fn a_header_with_no_data_is_a_valid_empty_program() {
        let mut file = b"EMPTY".to_vec();
        file.resize(22, 0);
        let atm = AtmFile::parse(&file).unwrap();
        assert!(atm.data.is_empty());
        assert_eq!(atm.name, "EMPTY");
        assert!(
            AtmFile::parse(&file[..21]).is_err(),
            "one byte short of a header"
        );
    }
}
