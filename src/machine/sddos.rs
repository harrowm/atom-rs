//! SDDOS: disk images stored on the SD card, behaving as floppy disks.
//!
//! SDDOS is a disk filing system by Kees van Oss that lives in the AtoMMC's
//! ROM image. Instead of talking to a floppy controller it asks the AtoMMC
//! card for sectors of a disk image file: `*DIN 0 GAME.SSD` mounts the file
//! `GAME.SSD` as drive 0, and then `*.`, `*LOAD` and the rest work on it as
//! they would on a floppy.
//!
//! This file is the card's half: up to four mounted images, and reading and
//! writing 256-byte sectors in them. (The ROM does everything else.) The
//! commands that reach it are in `atommc.rs`.
//!
//! A mounted image is remembered between runs in a small file on the card,
//! `BOOTDRV.CFG`, so the drives are still there next time. It holds four
//! 14-byte records (a 13-byte name and an attribute byte), as on the real
//! card; a drive with nothing mounted is all `FF`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A sector is 256 bytes, as on an Atom floppy.
pub const SECTOR_SIZE: usize = 256;
pub const DRIVE_COUNT: usize = 4;
/// The longest image name (the card keeps 13 bytes, including the end).
pub const NAME_LENGTH: usize = 12;
const RECORD_SIZE: usize = 14;
/// The saved list of mounted images, in the card's root folder.
pub const CONFIG_FILE: &str = "BOOTDRV.CFG";

/// An attribute byte of `FF` means "nothing mounted". Otherwise it holds the
/// FAT file attributes: bit 0 is read-only, and an ordinary file has `20`
/// (the archive bit). The ROM treats a reply of `40` or more as an error.
pub const NOT_MOUNTED: u8 = 0xFF;
const ARCHIVE: u8 = 0x20;
const READ_ONLY: u8 = 0x01;

/// Errors (added to the "complete" status `40` by the caller).
pub mod error {
    pub const INVALID_DRIVE: u8 = 0x09;
    pub const READ_ONLY: u8 = 0x0A;
    pub const ALREADY_MOUNTED: u8 = 0x0A;
    pub const DISK_ERROR: u8 = 0x01;
    pub const NO_FILE: u8 = 0x04;
}

struct Mounted {
    name: String,
    file: File,
    attributes: u8,
}

#[derive(Default)]
pub struct Drives {
    drives: [Option<Mounted>; DRIVE_COUNT],
    /// Where the next sector read or write goes (set by `load_position`).
    current_drive: usize,
    sector: u32,
}

impl Drives {
    /// Open the file `path` as the image in `drive`, calling it `name`.
    /// Returns its attribute byte, or an error to add to the status.
    ///
    /// The image is opened for reading and writing; if the host file is
    /// read-only it is opened for reading only and marked read-only, so its
    /// writes are refused. (The real card cannot mount a read-only image.)
    pub fn mount(&mut self, drive: usize, name: &str, path: &Path) -> Result<u8, u8> {
        let drive = drive % DRIVE_COUNT;
        self.drives[drive] = None;
        // The same image cannot be in two drives at once.
        let wanted = name.to_ascii_uppercase();
        if self
            .drives
            .iter()
            .flatten()
            .any(|other| other.name.to_ascii_uppercase() == wanted)
        {
            return Err(error::ALREADY_MOUNTED);
        }
        if !path.is_file() {
            return Err(error::NO_FILE);
        }
        let (file, attributes) = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => (file, ARCHIVE),
            Err(_) => match File::open(path) {
                Ok(file) => (file, ARCHIVE | READ_ONLY),
                Err(_) => return Err(error::DISK_ERROR),
            },
        };
        self.drives[drive] = Some(Mounted {
            name: name.chars().take(NAME_LENGTH).collect(),
            file,
            attributes,
        });
        Ok(attributes)
    }

    pub fn unmount(&mut self, drive: usize) {
        self.drives[drive % DRIVE_COUNT] = None;
    }

    /// The attribute byte of a drive's image, or `FF` if nothing is mounted.
    pub fn status(&self, drive: usize) -> u8 {
        self.drives[drive % DRIVE_COUNT]
            .as_ref()
            .map_or(NOT_MOUNTED, |mounted| mounted.attributes)
    }

    /// The names of the four images (empty for an empty drive).
    pub fn names(&self) -> [String; DRIVE_COUNT] {
        std::array::from_fn(|drive| {
            self.drives[drive]
                .as_ref()
                .map_or_else(String::new, |mounted| mounted.name.clone())
        })
    }

    /// Choose the sector (counting 256-byte sectors from the start of the
    /// image) that the next read or write uses, and the drive it is in.
    pub fn load_position(&mut self, drive: usize, sector: u32) {
        self.current_drive = drive % DRIVE_COUNT;
        self.sector = sector;
    }

    /// Read the chosen sector into `buffer`. A sector past the end of the
    /// image reads as far as it goes, leaving the rest of `buffer` alone.
    pub fn read_sector(&mut self, buffer: &mut [u8; SECTOR_SIZE]) -> Result<(), u8> {
        let sector = self.sector;
        let mounted = self.drives[self.current_drive]
            .as_mut()
            .ok_or(error::INVALID_DRIVE)?;
        mounted
            .file
            .seek(SeekFrom::Start(u64::from(sector) * SECTOR_SIZE as u64))
            .map_err(|_| error::DISK_ERROR)?;
        let mut total = 0;
        while total < SECTOR_SIZE {
            match mounted.file.read(&mut buffer[total..]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(_) => return Err(error::DISK_ERROR),
            }
        }
        Ok(())
    }

    /// Write `buffer` to the chosen sector, growing the image if needed.
    pub fn write_sector(&mut self, buffer: &[u8; SECTOR_SIZE]) -> Result<(), u8> {
        let sector = self.sector;
        let mounted = self.drives[self.current_drive]
            .as_mut()
            .ok_or(error::INVALID_DRIVE)?;
        if mounted.attributes & READ_ONLY != 0 {
            return Err(error::READ_ONLY);
        }
        mounted
            .file
            .seek(SeekFrom::Start(u64::from(sector) * SECTOR_SIZE as u64))
            .map_err(|_| error::DISK_ERROR)?;
        mounted
            .file
            .write_all(buffer)
            .and_then(|()| mounted.file.sync_data())
            .map_err(|_| error::DISK_ERROR)
    }

    // ---- Remembering the drives -------------------------------------------------

    /// The contents of `BOOTDRV.CFG` for the drives as they are now.
    pub fn config_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(DRIVE_COUNT * RECORD_SIZE);
        for mounted in &self.drives {
            let mut record = [NOT_MOUNTED; RECORD_SIZE];
            if let Some(mounted) = mounted {
                record = [0; RECORD_SIZE];
                for (slot, byte) in record.iter_mut().zip(mounted.name.bytes()) {
                    *slot = byte;
                }
                record[RECORD_SIZE - 1] = mounted.attributes;
            }
            bytes.extend(record);
        }
        bytes
    }
}

/// The image names saved in a `BOOTDRV.CFG`, one per drive (`None` where
/// nothing was mounted). Bad or short data gives no drives.
pub fn names_in_config(bytes: &[u8]) -> [Option<String>; DRIVE_COUNT] {
    std::array::from_fn(|drive| {
        let record = bytes.get(drive * RECORD_SIZE..(drive + 1) * RECORD_SIZE)?;
        if record[RECORD_SIZE - 1] == NOT_MOUNTED {
            return None;
        }
        let name: String = record[..RECORD_SIZE - 1]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as char)
            .collect();
        (!name.is_empty()).then_some(name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch folder that is removed afterwards.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new() -> TempFolder {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "atom-rs-sddos-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }

        /// An image file in which byte `n` of sector `s` is `s + n`.
        fn image(&self, name: &str, sectors: usize) -> PathBuf {
            let path = self.0.join(name);
            let data: Vec<u8> = (0..sectors * SECTOR_SIZE)
                .map(|i| (i / SECTOR_SIZE + i) as u8)
                .collect();
            fs::write(&path, data).unwrap();
            path
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mounting_gives_the_file_attributes_and_nothing_is_mounted_before() {
        let folder = TempFolder::new();
        let path = folder.image("A.SSD", 4);
        let mut drives = Drives::default();
        assert_eq!(drives.status(0), NOT_MOUNTED);
        assert_eq!(drives.mount(0, "A.SSD", &path), Ok(0x20));
        assert_eq!(drives.status(0), 0x20);
        assert_eq!(drives.status(1), NOT_MOUNTED);
    }

    #[test]
    fn a_missing_image_cannot_be_mounted() {
        let folder = TempFolder::new();
        let mut drives = Drives::default();
        assert_eq!(
            drives.mount(0, "NO.SSD", &folder.0.join("NO.SSD")),
            Err(error::NO_FILE)
        );
        assert_eq!(drives.status(0), NOT_MOUNTED);
    }

    #[test]
    fn reads_the_sector_asked_for() {
        let folder = TempFolder::new();
        let mut drives = Drives::default();
        drives.mount(0, "A.SSD", &folder.image("A.SSD", 4)).unwrap();
        let mut buffer = [0; SECTOR_SIZE];
        drives.load_position(0, 2);
        drives.read_sector(&mut buffer).unwrap();
        assert_eq!(buffer[0], 2, "sector 2 starts with 2 + 0");
        assert_eq!(buffer[5], 7);
        drives.load_position(0, 3);
        drives.read_sector(&mut buffer).unwrap();
        assert_eq!(buffer[0], 3);
    }

    #[test]
    fn reading_past_the_end_leaves_the_buffer_alone() {
        let folder = TempFolder::new();
        let mut drives = Drives::default();
        drives.mount(0, "A.SSD", &folder.image("A.SSD", 1)).unwrap();
        let mut buffer = [0xEE; SECTOR_SIZE];
        drives.load_position(0, 5);
        drives.read_sector(&mut buffer).unwrap();
        assert!(buffer.iter().all(|&b| b == 0xEE));
    }

    #[test]
    fn writes_a_sector_and_it_reads_back_and_is_in_the_file() {
        let folder = TempFolder::new();
        let path = folder.image("A.SSD", 2);
        let mut drives = Drives::default();
        drives.mount(0, "A.SSD", &path).unwrap();
        drives.load_position(0, 1);
        drives.write_sector(&[0x77; SECTOR_SIZE]).unwrap();
        let mut buffer = [0; SECTOR_SIZE];
        drives.read_sector(&mut buffer).unwrap();
        assert_eq!(buffer, [0x77; SECTOR_SIZE]);
        assert_eq!(
            &fs::read(&path).unwrap()[SECTOR_SIZE..],
            &[0x77; SECTOR_SIZE]
        );
    }

    #[test]
    fn writing_past_the_end_grows_the_image() {
        let folder = TempFolder::new();
        let path = folder.image("A.SSD", 1);
        let mut drives = Drives::default();
        drives.mount(0, "A.SSD", &path).unwrap();
        drives.load_position(0, 3);
        drives.write_sector(&[1; SECTOR_SIZE]).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 4 * SECTOR_SIZE as u64);
    }

    #[test]
    fn a_read_only_image_mounts_but_refuses_writes() {
        let folder = TempFolder::new();
        let path = folder.image("RO.SSD", 2);
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();

        let mut drives = Drives::default();
        assert_eq!(drives.mount(0, "RO.SSD", &path), Ok(0x21));
        drives.load_position(0, 0);
        assert!(drives.read_sector(&mut [0; SECTOR_SIZE]).is_ok());
        assert_eq!(
            drives.write_sector(&[0; SECTOR_SIZE]),
            Err(error::READ_ONLY)
        );

        let mut permissions = fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
    }

    #[test]
    fn the_same_image_cannot_be_mounted_twice_but_can_be_moved() {
        let folder = TempFolder::new();
        let path = folder.image("A.SSD", 1);
        let mut drives = Drives::default();
        drives.mount(0, "A.SSD", &path).unwrap();
        assert_eq!(
            drives.mount(1, "a.ssd", &path),
            Err(error::ALREADY_MOUNTED),
            "case is ignored"
        );
        // Mounting into the same drive again replaces it, which is fine.
        assert!(drives.mount(0, "A.SSD", &path).is_ok());
    }

    #[test]
    fn an_empty_drive_gives_an_error_not_data() {
        let mut drives = Drives::default();
        drives.load_position(2, 0);
        assert_eq!(
            drives.read_sector(&mut [0; SECTOR_SIZE]),
            Err(error::INVALID_DRIVE)
        );
        assert_eq!(
            drives.write_sector(&[0; SECTOR_SIZE]),
            Err(error::INVALID_DRIVE)
        );
    }

    #[test]
    fn unmounting_empties_the_drive() {
        let folder = TempFolder::new();
        let mut drives = Drives::default();
        drives.mount(1, "A.SSD", &folder.image("A.SSD", 1)).unwrap();
        drives.unmount(1);
        assert_eq!(drives.status(1), NOT_MOUNTED);
        assert_eq!(drives.names()[1], "");
    }

    #[test]
    fn the_drive_numbers_wrap_to_four() {
        let folder = TempFolder::new();
        let mut drives = Drives::default();
        drives.mount(5, "A.SSD", &folder.image("A.SSD", 1)).unwrap();
        assert_eq!(drives.status(1), 0x20, "drive 5 is drive 1");
    }

    #[test]
    fn the_config_holds_four_records_and_reads_back_the_names() {
        let folder = TempFolder::new();
        let mut drives = Drives::default();
        drives
            .mount(0, "ONE.SSD", &folder.image("ONE.SSD", 1))
            .unwrap();
        drives
            .mount(2, "THREE.DSD", &folder.image("THREE.DSD", 1))
            .unwrap();
        let config = drives.config_bytes();
        assert_eq!(config.len(), 56);
        assert_eq!(
            names_in_config(&config),
            [
                Some("ONE.SSD".to_string()),
                None,
                Some("THREE.DSD".to_string()),
                None
            ]
        );
        assert_eq!(&config[14..28], &[0xFF; 14], "an empty drive is all FF");
        assert_eq!(config[13], 0x20, "the attribute byte ends the record");
    }

    #[test]
    fn short_or_empty_config_data_gives_no_drives() {
        assert_eq!(names_in_config(&[]), [None, None, None, None]);
        assert_eq!(names_in_config(&[0; 10]), [None, None, None, None]);
    }
}
