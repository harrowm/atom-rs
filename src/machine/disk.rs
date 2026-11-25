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
    fn rejects_tiny_files() {
        assert!(DiskImage::from_bytes(vec![0; 100], 1).is_err());
    }
}
