//! Floppy disk images.
//!
//! The Atom's disk system stores 256-byte sectors, 10 per track, in a layout
//! shared with the BBC Micro's DFS:
//!
//! * `.ssd`: one side; the sectors simply follow each other, track by track.
//! * `.dsd`: two sides, interleaved track by track (track 0 side 0, track 0
//!   side 1, track 1 side 0, ...).
//!
//! Sector 0 and 1 of track 0 hold the catalogue (file names and where they
//! are on the disk).

use serde::{Deserialize, Serialize};

pub const SECTOR_SIZE: usize = 256;
pub const SECTORS_PER_TRACK: usize = 10;
const TRACK_SIZE: usize = SECTOR_SIZE * SECTORS_PER_TRACK;
/// A standard 80 track disk side holds this many sectors.
const SECTORS_PER_80_TRACK_SIDE: usize = 800;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiskImage {
    data: Vec<u8>,
    pub sides: u8,
    pub tracks: u8,
    pub write_protected: bool,
    /// Has the disk been written to since it was loaded or last saved?
    pub modified: bool,
}

impl DiskImage {
    /// Interpret the bytes of an `.ssd` (one side) or `.dsd` (two sides) file.
    pub fn from_bytes(data: Vec<u8>, sides: u8) -> Result<Self, String> {
        if !(1..=2).contains(&sides) {
            return Err("a disk has one or two sides".to_string());
        }
        if data.len() < 2 * SECTOR_SIZE {
            return Err("disk image is too small to contain a catalogue".to_string());
        }
        let tracks = data.len().div_ceil(TRACK_SIZE * sides as usize).min(255) as u8;
        Ok(DiskImage {
            data,
            sides,
            tracks,
            write_protected: false,
            modified: false,
        })
    }

    /// An empty, formatted disk with a valid (empty) DFS catalogue.
    pub fn blank(sides: u8, tracks: u8) -> Self {
        let size = tracks as usize * TRACK_SIZE * sides as usize;
        let mut data = vec![0; size];
        // Sector 1 of the catalogue records how many sectors the disk has
        // (bytes 6-7; the high bits share byte 6 with the boot option).
        let sectors = tracks as usize * SECTORS_PER_TRACK;
        data[SECTOR_SIZE + 6] = (sectors >> 8) as u8 & 3;
        data[SECTOR_SIZE + 7] = sectors as u8;
        DiskImage {
            data,
            sides,
            tracks,
            write_protected: false,
            modified: false,
        }
    }

    /// A standard 80 track blank disk.
    pub fn blank_80_track(sides: u8) -> Self {
        debug_assert_eq!(SECTORS_PER_80_TRACK_SIDE, 80 * SECTORS_PER_TRACK);
        Self::blank(sides, 80)
    }

    /// The bytes to write to an `.ssd`/`.dsd` file.
    pub fn to_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Where a sector lives in `data`, if the disk has it.
    fn offset(&self, track: u8, side: u8, sector: u8) -> Option<usize> {
        if side >= self.sides || sector as usize >= SECTORS_PER_TRACK {
            return None;
        }
        let track_number = track as usize * self.sides as usize + side as usize;
        let offset = (track_number * SECTORS_PER_TRACK + sector as usize) * SECTOR_SIZE;
        (offset + SECTOR_SIZE <= self.data.len()).then_some(offset)
    }

    pub fn read_sector(&self, track: u8, side: u8, sector: u8) -> Option<&[u8]> {
        let offset = self.offset(track, side, sector)?;
        Some(&self.data[offset..offset + SECTOR_SIZE])
    }

    /// Returns false if the sector does not exist.
    pub fn write_sector(&mut self, track: u8, side: u8, sector: u8, bytes: &[u8]) -> bool {
        let Some(offset) = self.offset(track, side, sector) else {
            return false;
        };
        self.data[offset..offset + SECTOR_SIZE].copy_from_slice(bytes);
        self.modified = true;
        true
    }

    /// Fill every sector of a track with a byte (what formatting does).
    pub fn format_track(&mut self, track: u8, side: u8, fill: u8) {
        for sector in 0..SECTORS_PER_TRACK as u8 {
            if let Some(offset) = self.offset(track, side, sector) {
                self.data[offset..offset + SECTOR_SIZE].fill(fill);
                self.modified = true;
            }
        }
    }
}

/// One entry of the disk catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub name: String,
    pub load_address: u32,
    pub exec_address: u32,
    pub length: u32,
    pub start_sector: u16,
}

/// A DFS catalogue holds at most this many files.
const MAX_FILES: usize = 31;
const CATALOGUE_ENTRY_SIZE: usize = 8;
const FIRST_FILE_SECTOR: u16 = 2;

impl DiskImage {
    /// The files listed in the catalogue (side 0).
    ///
    /// Sector 0 holds each file's name (7 characters plus a directory
    /// character); sector 1 holds where it loads and runs, how long it is and
    /// where on the disk it starts.
    pub fn catalogue(&self) -> Vec<CatalogueEntry> {
        let (Some(names), Some(info)) = (self.read_sector(0, 0, 0), self.read_sector(0, 0, 1))
        else {
            return Vec::new();
        };
        let count = (info[5] as usize / CATALOGUE_ENTRY_SIZE).min(MAX_FILES);
        (0..count)
            .map(|i| {
                let at = 8 + i * CATALOGUE_ENTRY_SIZE;
                let name: String = names[at..at + 7]
                    .iter()
                    .map(|&b| (b & 0x7F) as char)
                    .collect();
                let e = &info[at..at + CATALOGUE_ENTRY_SIZE];
                // Byte 6 packs the top bits of four 18-bit values.
                let high = e[6] as u32;
                CatalogueEntry {
                    name: name.trim_end().to_string(),
                    load_address: u16::from_le_bytes([e[0], e[1]]) as u32 | ((high >> 2) & 3) << 16,
                    exec_address: u16::from_le_bytes([e[2], e[3]]) as u32 | ((high >> 6) & 3) << 16,
                    length: u16::from_le_bytes([e[4], e[5]]) as u32 | ((high >> 4) & 3) << 16,
                    start_sector: ((high & 3) << 8 | e[7] as u32) as u16,
                }
            })
            .collect()
    }

    /// Add a file to the disk and its catalogue, like saving it would.
    /// `name` is up to 7 characters; the directory is the default (a space).
    pub fn add_file(
        &mut self,
        name: &str,
        load_address: u32,
        exec_address: u32,
        data: &[u8],
    ) -> Result<(), String> {
        let name = name.to_ascii_uppercase();
        if name.is_empty() || name.len() > 7 || !name.is_ascii() {
            return Err(format!(
                "{name:?} is not a valid file name (1-7 ASCII characters)"
            ));
        }
        let files = self.catalogue();
        if files.iter().any(|f| f.name == name) {
            return Err(format!("{name} is already on the disk"));
        }
        if files.len() >= MAX_FILES {
            return Err(format!("the catalogue is full ({MAX_FILES} files)"));
        }

        // New files go after the last used sector.
        let start = files
            .iter()
            .map(|f| f.start_sector + (f.length as usize).div_ceil(SECTOR_SIZE) as u16)
            .max()
            .unwrap_or(FIRST_FILE_SECTOR)
            .max(FIRST_FILE_SECTOR);
        let sectors_needed = data.len().div_ceil(SECTOR_SIZE) as u16;
        let sectors_on_disk = self.tracks as u16 * SECTORS_PER_TRACK as u16;
        if start + sectors_needed > sectors_on_disk {
            return Err(format!("no room for {name}: the disk is full"));
        }

        // Write the data, one sector at a time.
        for (i, chunk) in data.chunks(SECTOR_SIZE).enumerate() {
            let sector = start as usize + i;
            let mut bytes = [0u8; SECTOR_SIZE];
            bytes[..chunk.len()].copy_from_slice(chunk);
            let (track, sector) = (sector / SECTORS_PER_TRACK, sector % SECTORS_PER_TRACK);
            self.write_sector(track as u8, 0, sector as u8, &bytes);
        }

        // Catalogue entries are kept newest first, so make room at the front.
        let mut names = self
            .read_sector(0, 0, 0)
            .expect("catalogue exists")
            .to_vec();
        let mut info = self
            .read_sector(0, 0, 1)
            .expect("catalogue exists")
            .to_vec();
        let used = files.len() * CATALOGUE_ENTRY_SIZE;
        names.copy_within(8..8 + used, 8 + CATALOGUE_ENTRY_SIZE);
        info.copy_within(8..8 + used, 8 + CATALOGUE_ENTRY_SIZE);

        let mut padded = [b' '; CATALOGUE_ENTRY_SIZE];
        padded[..name.len()].copy_from_slice(name.as_bytes());
        names[8..16].copy_from_slice(&padded);

        let length = data.len() as u32;
        info[8..16].copy_from_slice(&[
            load_address as u8,
            (load_address >> 8) as u8,
            exec_address as u8,
            (exec_address >> 8) as u8,
            length as u8,
            (length >> 8) as u8,
            ((start >> 8) as u8 & 3)
                | (((load_address >> 16) as u8 & 3) << 2)
                | (((length >> 16) as u8 & 3) << 4)
                | (((exec_address >> 16) as u8 & 3) << 6),
            start as u8,
        ]);
        info[5] = (files.len() as u8 + 1) * CATALOGUE_ENTRY_SIZE as u8;

        self.write_sector(0, 0, 0, &names);
        self.write_sector(0, 0, 1, &info);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_sided_sectors_follow_each_other() {
        let mut data = vec![0; 3 * TRACK_SIZE];
        data[(TRACK_SIZE + 2 * SECTOR_SIZE) + 5] = 0x77; // track 1, sector 2, byte 5
        let disk = DiskImage::from_bytes(data, 1).unwrap();
        assert_eq!(disk.tracks, 3);
        assert_eq!(disk.read_sector(1, 0, 2).unwrap()[5], 0x77);
        assert!(disk.read_sector(3, 0, 0).is_none(), "beyond the last track");
        assert!(disk.read_sector(0, 1, 0).is_none(), "no second side");
        assert!(disk.read_sector(0, 0, 10).is_none(), "only sectors 0-9");
    }

    #[test]
    fn double_sided_images_interleave_tracks() {
        let mut data = vec![0; 4 * TRACK_SIZE]; // 2 tracks, 2 sides
        data[TRACK_SIZE] = 0xAA; // second track-sized chunk = track 0, side 1
        data[2 * TRACK_SIZE] = 0xBB; // third = track 1, side 0
        let disk = DiskImage::from_bytes(data, 2).unwrap();
        assert_eq!(disk.read_sector(0, 1, 0).unwrap()[0], 0xAA);
        assert_eq!(disk.read_sector(1, 0, 0).unwrap()[0], 0xBB);
    }

    #[test]
    fn writing_changes_the_sector_and_marks_the_disk_modified() {
        let mut disk = DiskImage::blank_80_track(1);
        assert!(!disk.modified);
        assert!(disk.write_sector(5, 0, 3, &[9; SECTOR_SIZE]));
        assert!(disk.modified);
        assert_eq!(disk.read_sector(5, 0, 3).unwrap()[100], 9);
        assert!(!disk.write_sector(80, 0, 0, &[0; SECTOR_SIZE]));
    }

    #[test]
    fn blank_disk_catalogue_records_its_size() {
        let disk = DiskImage::blank_80_track(1);
        let sector1 = disk.read_sector(0, 0, 1).unwrap();
        assert_eq!(((sector1[6] as usize & 3) << 8) | sector1[7] as usize, 800);
        assert_eq!(disk.to_bytes().len(), 80 * TRACK_SIZE);
    }

    #[test]
    fn added_files_appear_in_the_catalogue_with_their_data() {
        let mut disk = DiskImage::blank_80_track(1);
        let first: Vec<u8> = (0..300u32).map(|i| i as u8).collect(); // 2 sectors
        disk.add_file("hello", 0x2900, 0xC2B2, &first).unwrap();
        disk.add_file("DATA", 0x3000, 0x3000, &[7; 10]).unwrap();

        let files = disk.catalogue();
        assert_eq!(files.len(), 2);
        // Newest first.
        assert_eq!(files[0].name, "DATA");
        assert_eq!((files[0].load_address, files[0].length), (0x3000, 10));
        assert_eq!(files[1].name, "HELLO");
        assert_eq!(
            (files[1].load_address, files[1].exec_address),
            (0x2900, 0xC2B2)
        );
        assert_eq!(files[1].length, 300);
        // HELLO takes sectors 2-3, so DATA starts at 4.
        assert_eq!((files[1].start_sector, files[0].start_sector), (2, 4));
        assert_eq!(disk.read_sector(0, 0, 3).unwrap()[43], first[256 + 43]);
        assert_eq!(disk.read_sector(0, 0, 4).unwrap()[9], 7);
    }

    #[test]
    fn add_file_refuses_bad_names_duplicates_and_overflow() {
        let mut disk = DiskImage::blank(1, 2); // only 20 sectors
        assert!(disk.add_file("TOOLONGNAME", 0, 0, &[1]).is_err());
        assert!(disk.add_file("", 0, 0, &[1]).is_err());
        disk.add_file("A", 0, 0, &[1]).unwrap();
        assert!(
            disk.add_file("a", 0, 0, &[1])
                .unwrap_err()
                .contains("already")
        );
        assert!(
            disk.add_file("BIG", 0, 0, &[0; 20 * 256])
                .unwrap_err()
                .contains("full")
        );
    }

    #[test]
    fn rejects_tiny_files() {
        assert!(DiskImage::from_bytes(vec![0; 100], 1).is_err());
    }
}
