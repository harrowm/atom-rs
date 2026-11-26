//! AtoMMC: an SD card interface for the Atom, with a folder on your computer
//! standing in for the card.
//!
//! AtoMMC is a modern add-on. A small microcontroller on it reads an SD card
//! and talks to the Atom through a few registers at `B400`. The Atom's AtoMMC
//! ROM (which replaces the Atom DOS ROM) provides commands such as `*CAT`,
//! `*LOAD`, `*SAVE`, `*CWD` and `*MENU` on top of them. The Atom Software
//! Archive is laid out as the contents of such a card.
//!
//! ## The registers
//!
//! | Address | Write | Read |
//! |---------|-------|------|
//! | `B400` | a command | the reply (a status, or a byte of data) |
//! | `B401` | a value, held in the "latch" for the next command | |
//! | `B402` | | the next byte of data (after `INIT_READ`) |
//! | `B403` | the next byte of data to send (after `INIT_WRITE`) | |
//!
//! The Atom writes a command and polls for a reply other than "busy" (`80`).
//! Filenames and other data go through a 256 byte buffer: written byte by byte
//! to `B403`, or read byte by byte from `B402`.
//!
//! ## Replies
//!
//! `3F` is "OK". `40 + n` is "complete", where `n` is a FatFs-style error code
//! (0 = no error, 4 = file not found, 5 = path not found, and so on), and the
//! ROM turns those into messages like `NOT FOUND`. `60` means end of file.
//!
//! ## What is emulated
//!
//! With no folder chosen, only the joystick port commands answer (some games
//! read their joystick that way) and everything else reads as open bus, as if
//! no AtoMMC were fitted. With a folder, the directory and file commands work
//! on it. File and folder names are matched without regard to upper or lower
//! case and must be valid 8.3 names, as on a real card; nothing outside the
//! chosen folder can be reached. Not emulated yet: SDDOS (disk images stored
//! on the card).
//!
//! This follows Atomulator's port of the AtoMMC firmware.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::memory::open_bus;
use super::sddos::{self, Drives};

/// Register numbers (the low three address bits).
mod register {
    pub const COMMAND: u8 = 0;
    pub const LATCH: u8 = 1;
    pub const READ_DATA: u8 = 2;
    pub const WRITE_DATA: u8 = 3;
}

/// Commands written to the command register.
mod command {
    pub const DIR_OPEN: u8 = 0x00;
    pub const DIR_READ: u8 = 0x01;
    pub const DIR_CWD: u8 = 0x02;
    pub const DIR_MKDIR: u8 = 0x04;
    pub const DIR_RMDIR: u8 = 0x05;
    pub const RENAME: u8 = 0x08;

    // The file group. Bits 5-6 of the command carry a file number, which is
    // removed before these are compared.
    pub const FILE_CLOSE: u8 = 0x10;
    pub const FILE_OPEN_READ: u8 = 0x11;
    pub const FILE_OPEN_WRITE: u8 = 0x13;
    pub const FILE_DELETE: u8 = 0x14;
    pub const FILE_GET_INFO: u8 = 0x15;
    pub const FILE_SEEK: u8 = 0x16;
    pub const FILE_OPEN_RANDOM_ACCESS: u8 = 0x17;
    /// Mount a disk image as a drive, for SDDOS (see `sddos.rs`).
    pub const FILE_OPEN_IMAGE: u8 = 0x12;

    // The data group (bits 2-3 carry a file number).
    pub const INIT_READ: u8 = 0x20;
    pub const INIT_WRITE: u8 = 0x21;
    pub const READ_BYTES: u8 = 0x22;
    pub const WRITE_BYTES: u8 = 0x23;

    pub const EXEC_PACKET: u8 = 0x3F;

    // The SDDOS group: disk images used as drives (see `sddos.rs`).
    pub const LOAD_POSITION: u8 = 0x40;
    pub const GET_IMAGE_STATUS: u8 = 0x41;
    pub const GET_IMAGE_NAMES: u8 = 0x42;
    pub const READ_IMAGE_SECTOR: u8 = 0x43;
    pub const WRITE_IMAGE_SECTOR: u8 = 0x44;
    pub const SAVE_IMAGE_LIST: u8 = 0x45;
    pub const RESTORE_IMAGE_LIST: u8 = 0x46;
    pub const UNMOUNT_IMAGE: u8 = 0x47;

    pub const GET_CARD_TYPE: u8 = 0x80;
    pub const GET_PORT_DDR: u8 = 0xA0;
    pub const SET_PORT_DDR: u8 = 0xA1;
    pub const READ_PORT: u8 = 0xA2;
    pub const WRITE_PORT: u8 = 0xA3;
    pub const GET_FIRMWARE_VERSION: u8 = 0xE0;
    pub const GET_BOOTLOADER_VERSION: u8 = 0xE1;
    pub const GET_CONFIG_BYTE: u8 = 0xF0;
    pub const SET_CONFIG_BYTE: u8 = 0xF1;
    pub const READ_AUX: u8 = 0xFD;
    pub const GET_HEARTBEAT: u8 = 0xFE;
}

/// Replies.
mod status {
    pub const OK: u8 = 0x3F;
    pub const COMPLETE: u8 = 0x40;
    pub const END_OF_FILE: u8 = 0x60;
    pub const BUSY: u8 = 0x80;
}

/// FatFs error codes, which the AtoMMC ROM turns into messages.
mod error {
    pub const NONE: u8 = 0;
    pub const DISK_ERROR: u8 = 1;
    pub const NO_FILE: u8 = 4;
    pub const NO_PATH: u8 = 5;
    pub const INVALID_NAME: u8 = 6;
    pub const DENIED: u8 = 7;
    pub const EXISTS: u8 = 8;
    pub const INVALID_OBJECT: u8 = 9;
    /// Used by the firmware for "no data in the buffer".
    pub const NO_DATA: u8 = 0x08;
    pub const TOO_MANY_OPEN: u8 = 0x12;
}

/// Open files (other than the fixed scratch file 0) are reported to the Atom
/// as `0x20 + file number`, to tell them apart from errors.
const FILE_NUMBER_OFFSET: u8 = 0x20;
/// The scratch file plus three random-access files.
const FILE_SLOTS: usize = 4;
/// The firmware reports itself as version 2.13.
const FIRMWARE_VERSION: u8 = 2 << 4 | 13;
/// Where the configuration byte lives in the 1K EEPROM.
const EEPROM_CONFIG_BYTE: usize = 0xFF;
const EEPROM_SIZE: usize = 1024;
/// FAT directory attribute bit.
const ATTRIBUTE_DIRECTORY: u8 = 0x10;
/// Longest wildcard pattern the firmware keeps.
const WILDCARD_LENGTH: usize = 16;

// ---- The card (a folder on the host) ------------------------------------------

/// One entry of a directory listing.
#[derive(Clone, Debug)]
struct Entry {
    name: String,
    is_directory: bool,
    size: u32,
}

struct OpenFile {
    file: File,
    attribute: u8,
}

/// The folder standing in for the SD card, and what is open on it.
struct Card {
    root: PathBuf,
    /// The current directory, as names from the root down.
    cwd: Vec<String>,
    files: [Option<OpenFile>; FILE_SLOTS],
    listing: Vec<Entry>,
    next_entry: usize,
    wildcard: String,
}

type FsResult<T> = Result<T, u8>;

impl Card {
    fn new(root: PathBuf) -> Card {
        Card {
            root,
            cwd: Vec::new(),
            files: Default::default(),
            listing: Vec::new(),
            next_entry: 0,
            wildcard: "*".to_string(),
        }
    }

    /// Back to the root, with nothing open (what a reset does).
    fn reset(&mut self) {
        self.cwd.clear();
        self.files = Default::default();
        self.listing.clear();
        self.next_entry = 0;
    }

    fn host_path(&self, components: &[String]) -> PathBuf {
        components
            .iter()
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    /// Split `path` into the directory it names (as components below the
    /// root) and a final name, following `.` and `..` but never leaving the
    /// root. Every part but the last must be an existing directory.
    fn split_path(&self, path: &str) -> FsResult<(Vec<String>, String)> {
        let normalised = path.replace('\\', "/");
        let mut directory = if normalised.starts_with('/') {
            Vec::new()
        } else {
            self.cwd.clone()
        };
        let parts: Vec<&str> = normalised
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        let Some((last, middle)) = parts.split_last() else {
            return Ok((directory, String::new()));
        };
        for part in middle {
            self.step_into(&mut directory, part)?;
        }
        Ok((directory, last.to_string()))
    }

    /// Move `directory` into the sub-directory `name` (or up, for `..`).
    fn step_into(&self, directory: &mut Vec<String>, name: &str) -> FsResult<()> {
        if name == ".." {
            directory.pop();
            return Ok(());
        }
        match find_entry(&self.host_path(directory), name) {
            Some((actual, true)) => {
                directory.push(actual);
                Ok(())
            }
            _ => Err(error::NO_PATH),
        }
    }

    /// The components of an existing directory.
    fn resolve_directory(&self, path: &str) -> FsResult<Vec<String>> {
        let (mut directory, last) = self.split_path(path)?;
        if !last.is_empty() {
            self.step_into(&mut directory, &last)?;
        }
        Ok(directory)
    }

    /// Where a file lives on the host. `name` must be a valid 8.3 name. If the
    /// file exists its real spelling is used; otherwise the name is made upper
    /// case, as on the Atom.
    fn resolve_file(&self, path: &str) -> FsResult<(PathBuf, bool)> {
        let (directory, last) = self.split_path(path)?;
        let name = validate_name(&last)?;
        let host_directory = self.host_path(&directory);
        match find_entry(&host_directory, &name) {
            Some((actual, is_directory)) => Ok((host_directory.join(actual), is_directory)),
            None => Ok((host_directory.join(name.to_ascii_uppercase()), false)),
        }
    }

    // ---- Directories ---------------------------------------------------------

    /// Start listing the directory in `path`; a wildcard may end the path.
    fn open_directory(&mut self, path: &str) -> FsResult<()> {
        let (directory_path, pattern) = split_wildcard(path);
        let components = self.resolve_directory(&directory_path)?;
        let mut listing = Vec::new();
        for item in fs::read_dir(self.host_path(&components))
            .map_err(|_| error::NO_PATH)?
            .flatten()
        {
            let Ok(name) = item.file_name().into_string() else {
                continue;
            };
            if !is_short_name(&name) {
                continue;
            }
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            listing.push(Entry {
                name,
                is_directory: metadata.is_dir(),
                size: metadata.len().min(u32::MAX as u64) as u32,
            });
        }
        listing.sort_by_key(|entry| entry.name.to_ascii_uppercase());
        self.listing = listing;
        self.next_entry = 0;
        self.wildcard = pattern;
        Ok(())
    }

    /// The next listed entry matching the wildcard.
    fn read_directory(&mut self) -> Option<Entry> {
        while let Some(entry) = self.listing.get(self.next_entry).cloned() {
            self.next_entry += 1;
            if wildcard_matches(&self.wildcard, &entry.name) {
                return Some(entry);
            }
        }
        None
    }

    fn change_directory(&mut self, path: &str) -> FsResult<()> {
        self.cwd = self.resolve_directory(path)?;
        Ok(())
    }

    fn make_directory(&mut self, path: &str) -> FsResult<()> {
        let (host_path, exists) = self.resolve_file(path)?;
        if exists || host_path.exists() {
            return Err(error::EXISTS);
        }
        fs::create_dir(host_path).map_err(io_error_code)
    }

    /// Delete a file or an empty directory.
    fn delete(&mut self, path: &str) -> FsResult<()> {
        let (host_path, is_directory) = self.resolve_file(path)?;
        if !host_path.exists() {
            return Err(error::NO_PATH);
        }
        let result = if is_directory {
            fs::remove_dir(&host_path)
        } else {
            fs::remove_file(&host_path)
        };
        result.map_err(|_| error::DENIED)
    }

    fn rename(&mut self, from: &str, to: &str) -> FsResult<()> {
        let (from_path, _) = self.resolve_file(from)?;
        let (to_path, _) = self.resolve_file(to)?;
        fs::rename(from_path, to_path).map_err(io_error_code)
    }

    // ---- Files -----------------------------------------------------------------

    /// Open a file into a slot and return the code to reply with. File 0 is
    /// the fixed scratch file; for any other number the first free slot is
    /// used and its number is returned.
    fn open_file(&mut self, requested_slot: usize, path: &str, how: OpenHow) -> (usize, u8) {
        let slot = if requested_slot == 0 {
            0
        } else {
            match (1..FILE_SLOTS).find(|&slot| self.files[slot].is_none()) {
                Some(free) => free,
                None => return (0, error::TOO_MANY_OPEN),
            }
        };
        match self.open_into(slot, path, how) {
            Ok(()) if requested_slot == 0 => (slot, error::NONE),
            Ok(()) => (slot, FILE_NUMBER_OFFSET | slot as u8),
            Err(code) => (slot, code),
        }
    }

    fn open_into(&mut self, slot: usize, path: &str, how: OpenHow) -> FsResult<()> {
        let (host_path, is_directory) = self.resolve_file(path)?;
        let exists = host_path.exists() && !is_directory;
        let mut options = OpenOptions::new();
        match how {
            OpenHow::ExistingForReading => {
                if !exists {
                    return Err(error::NO_FILE);
                }
                options.read(true);
            }
            OpenHow::NewForWriting => {
                if exists {
                    return Err(error::EXISTS);
                }
                options.write(true).create_new(true);
            }
            OpenHow::ForRandomAccess => {
                options.read(true).write(true).create(true);
            }
        }
        let file = options.open(host_path).map_err(io_error_code)?;
        self.files[slot] = Some(OpenFile { file, attribute: 0 });
        Ok(())
    }

    fn file(&mut self, slot: usize) -> FsResult<&mut OpenFile> {
        self.files[slot].as_mut().ok_or(error::INVALID_OBJECT)
    }
}

#[derive(Clone, Copy)]
enum OpenHow {
    ExistingForReading,
    NewForWriting,
    ForRandomAccess,
}

// ---- Name handling --------------------------------------------------------------

/// Find `name` in a host directory, ignoring case. Returns its real spelling
/// and whether it is a directory.
fn find_entry(directory: &Path, name: &str) -> Option<(String, bool)> {
    for item in fs::read_dir(directory).ok()?.flatten() {
        let Ok(actual) = item.file_name().into_string() else {
            continue;
        };
        if actual.eq_ignore_ascii_case(name) {
            return Some((actual, item.metadata().ok()?.is_dir()));
        }
    }
    None
}

/// Check a file name is a valid 8.3 name (1-8 characters, then optionally a
/// dot and up to 3 more). A dangling dot is dropped.
fn validate_name(name: &str) -> FsResult<String> {
    let (stem, suffix) = match name.split_once('.') {
        Some((stem, suffix)) => (stem, Some(suffix)),
        None => (name, None),
    };
    if stem.is_empty() || stem.len() > 8 || !name.is_ascii() {
        return Err(error::INVALID_NAME);
    }
    match suffix {
        None | Some("") => Ok(stem.to_string()),
        Some(suffix) if suffix.len() <= 3 && !suffix.contains('.') => Ok(name.to_string()),
        Some(_) => Err(error::INVALID_NAME),
    }
}

/// Names the card can show: at most 8 characters, a dot, at most 3 more.
fn is_short_name(name: &str) -> bool {
    if !name.is_ascii() {
        return false;
    }
    match name.split_once('.') {
        Some((stem, suffix)) => stem.len() <= 8 && suffix.len() <= 3,
        None => name.len() <= 8,
    }
}

/// Separate a directory path from a trailing wildcard: `LIB/A*` becomes
/// `("LIB", "A*")`, a plain path becomes `(path, "*")`.
fn split_wildcard(path: &str) -> (String, String) {
    let Some(wild_at) = path.find(['?', '*']) else {
        return (path.to_string(), "*".to_string());
    };
    let last_slash = path[..wild_at].rfind(['/', '\\']);
    let (directory, pattern) = match last_slash {
        Some(slash) => (&path[..slash], &path[slash + 1..]),
        None => ("", path),
    };
    (
        directory.to_string(),
        pattern.chars().take(WILDCARD_LENGTH).collect(),
    )
}

/// Does `name` match the pattern, where `*` is any run of characters and `?`
/// any one? Case is ignored.
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    fn matches(pattern: &[u8], name: &[u8]) -> bool {
        match (pattern.split_first(), name.split_first()) {
            (None, None) => true,
            (Some((b'*', rest)), _) => {
                matches(rest, name) || (!name.is_empty() && matches(pattern, &name[1..]))
            }
            (Some((b'?', rest)), Some((_, name_rest))) => matches(rest, name_rest),
            (Some((p, rest)), Some((n, name_rest))) if p.eq_ignore_ascii_case(n) => {
                matches(rest, name_rest)
            }
            _ => false,
        }
    }
    matches(pattern.as_bytes(), name.as_bytes())
}

fn io_error_code(error: std::io::Error) -> u8 {
    match error.kind() {
        std::io::ErrorKind::NotFound => error::NO_PATH,
        std::io::ErrorKind::PermissionDenied => error::DENIED,
        std::io::ErrorKind::AlreadyExists => error::EXISTS,
        _ => error::DISK_ERROR,
    }
}

// ---- The device -------------------------------------------------------------------

pub struct AtoMmc {
    /// What the Atom reads back. `None` until something has been answered, so
    /// that a machine with no AtoMMC reads open bus.
    data_port: Option<u8>,
    /// The register last written; reads of the data port act on it.
    latched_address: u8,
    /// A value written to the latch register, used by the next command.
    value_latch: u8,

    /// The firmware's 256 byte buffer for names and data.
    buffer: [u8; 256],
    index: u8,
    amount: u16,
    data_present: bool,
    /// The file number carried by the current command.
    file_number: usize,
    heartbeat: u8,

    eeprom: [u8; EEPROM_SIZE],
    port_direction: u8,
    port_output: u8,
    /// The joystick's pins: bits clear when pressed. `None` = no joystick,
    /// which reads as 0 (as in Atomulator).
    pub joystick: Option<u8>,

    card: Option<Card>,
    /// The disk images mounted as drives by SDDOS.
    drives: Drives,
}

impl Default for AtoMmc {
    fn default() -> Self {
        AtoMmc {
            data_port: None,
            latched_address: 0,
            value_latch: 0,
            buffer: [0; 256],
            index: 0,
            amount: 0,
            data_present: false,
            file_number: 0,
            heartbeat: 0x55,
            eeprom: [0xFF; EEPROM_SIZE],
            port_direction: 0,
            port_output: 0,
            joystick: None,
            card: None,
            drives: Drives::default(),
        }
    }
}

impl AtoMmc {
    /// An AtoMMC whose "card" is the folder `root`.
    pub fn with_card(root: PathBuf) -> AtoMmc {
        AtoMmc {
            card: Some(Card::new(root)),
            data_port: Some(0),
            ..AtoMmc::default()
        }
    }

    /// Use the 1K EEPROM contents from a file (it holds the configuration).
    pub fn load_eeprom(&mut self, bytes: &[u8]) {
        if bytes.len() == EEPROM_SIZE {
            self.eeprom.copy_from_slice(bytes);
        }
    }

    /// Take the card away: back to only the joystick port answering.
    pub fn remove_card(&mut self) {
        self.card = None;
        self.drives = Drives::default();
        self.reset();
    }

    pub fn card_folder(&self) -> Option<&Path> {
        self.card.as_ref().map(|card| card.root.as_path())
    }

    // ---- What the Atom does ----------------------------------------------------

    pub fn read(&mut self, addr: u16) -> u8 {
        let current = self.data_port.unwrap_or_else(|| open_bus(addr));
        if self.card.is_some() {
            self.process(None);
        }
        current
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        self.latched_address = (addr & 0x0F) as u8;
        if self.card.is_some() {
            self.process(Some(value));
        } else {
            self.process_without_card(value);
        }
    }

    /// The RESET line: files are closed and the current directory goes back
    /// to the root. The joystick, configuration and card stay.
    pub fn reset(&mut self) {
        if let Some(card) = &mut self.card {
            card.reset();
        }
        self.data_port = self.card.as_ref().map(|_| 0);
        self.latched_address = 0;
        self.value_latch = 0;
        self.index = 0;
        self.data_present = false;
    }

    /// With no card only the port commands answer.
    fn process_without_card(&mut self, value: u8) {
        match self.latched_address & 7 {
            register::LATCH => self.value_latch = value,
            register::COMMAND => {
                if matches!(
                    value,
                    command::GET_PORT_DDR
                        | command::SET_PORT_DDR
                        | command::READ_PORT
                        | command::WRITE_PORT
                ) {
                    self.run_command(value);
                } else {
                    self.data_port = None; // not understood: open bus
                }
            }
            _ => {}
        }
    }

    /// One access to the interface. `written` is the byte if it was a write.
    fn process(&mut self, written: Option<u8>) {
        match self.latched_address & 7 {
            register::COMMAND => {
                if let Some(received) = written {
                    self.run_command(received);
                }
            }
            // Any access while the data register is selected steps through the
            // buffer: the previous read prepared this byte, this one the next.
            register::READ_DATA => {
                self.data_port = Some(self.buffer[self.index as usize]);
                self.index = self.index.wrapping_add(1);
            }
            register::WRITE_DATA => {
                if let Some(received) = written {
                    self.buffer[self.index as usize] = received;
                    self.index = self.index.wrapping_add(1);
                    self.data_present = true;
                }
            }
            register::LATCH => {
                if let Some(received) = written {
                    self.value_latch = received;
                    self.data_port = Some(received);
                }
            }
            _ => {}
        }
    }

    // ---- Commands --------------------------------------------------------------

    fn run_command(&mut self, mut received: u8) {
        // Commands in the file and data groups carry a file number.
        if received & 0x98 == 0x10 {
            self.file_number = ((received >> 5) & 3) as usize;
            received &= 0x9F;
        }
        if received & 0xF0 == 0x20 {
            self.file_number = ((received >> 2) & 3) as usize;
            received &= 0xF3;
        }
        self.data_port = Some(status::BUSY);

        match received {
            command::INIT_READ => {
                // The first byte can be read at once.
                self.data_port = Some(self.buffer[0]);
                self.index = 1;
                self.latched_address = register::READ_DATA;
            }
            command::INIT_WRITE => {
                self.index = 0;
                self.data_present = false;
            }
            command::READ_BYTES => {
                self.amount = self.value_latch as u16;
                self.file_read();
            }
            command::WRITE_BYTES => {
                self.amount = self.value_latch as u16;
                self.file_write();
            }
            command::DIR_OPEN => self.directory_open(),
            command::DIR_READ => self.directory_read(),
            command::DIR_CWD => self.reply_complete_with(|card, name| card.change_directory(name)),
            command::DIR_MKDIR => self.reply_complete_with(|card, name| card.make_directory(name)),
            command::DIR_RMDIR | command::FILE_DELETE => {
                if received == command::FILE_DELETE {
                    // The ROM opens a file before deleting it; let go of it first.
                    let slot = self.file_number;
                    if let Some(card) = &mut self.card {
                        card.files[slot] = None;
                    }
                }
                self.reply_complete_with(|card, name| card.delete(name));
            }
            command::RENAME => self.rename(),
            command::FILE_OPEN_READ => self.file_open(OpenHow::ExistingForReading),
            command::FILE_OPEN_WRITE => self.file_open(OpenHow::NewForWriting),
            command::FILE_OPEN_RANDOM_ACCESS => self.file_open(OpenHow::ForRandomAccess),
            command::FILE_CLOSE => {
                let slot = self.file_number;
                if let Some(card) = &mut self.card {
                    card.files[slot] = None;
                }
                self.data_port = Some(status::COMPLETE);
            }
            command::FILE_GET_INFO => self.file_get_info(),
            command::FILE_SEEK => self.file_seek(),
            command::EXEC_PACKET => self.execute_packet(),

            command::FILE_OPEN_IMAGE => self.image_mount(),
            // This one gets no reply: the ROM does not wait for one.
            command::LOAD_POSITION => self.image_load_position(),
            command::GET_IMAGE_STATUS => {
                self.data_port = Some(self.drives.status(usize::from(self.value_latch)));
            }
            command::GET_IMAGE_NAMES => self.image_names(),
            command::READ_IMAGE_SECTOR => self.image_read_sector(),
            command::WRITE_IMAGE_SECTOR => self.image_write_sector(),
            command::SAVE_IMAGE_LIST => {
                self.save_image_list();
                self.data_port = Some(status::OK);
            }
            command::RESTORE_IMAGE_LIST => self.image_restore_list(),
            command::UNMOUNT_IMAGE => {
                self.drives.unmount(usize::from(self.value_latch));
                self.save_image_list();
                self.data_port = Some(status::OK);
            }

            command::GET_CARD_TYPE => self.data_port = Some(1), // "MMC"
            command::GET_PORT_DDR => self.data_port = Some(self.port_direction),
            command::SET_PORT_DDR => {
                self.port_direction = self.value_latch;
                self.data_port = Some(status::OK);
            }
            command::READ_PORT => self.data_port = Some(self.joystick.unwrap_or(0)),
            command::WRITE_PORT => {
                self.port_output = self.value_latch;
                self.data_port = Some(status::OK);
            }
            command::GET_FIRMWARE_VERSION => self.data_port = Some(FIRMWARE_VERSION),
            command::GET_BOOTLOADER_VERSION => self.data_port = Some(0),
            command::GET_CONFIG_BYTE => self.data_port = Some(self.eeprom[EEPROM_CONFIG_BYTE]),
            command::SET_CONFIG_BYTE => {
                self.eeprom[EEPROM_CONFIG_BYTE] = self.value_latch;
                self.data_port = Some(status::OK);
            }
            command::READ_AUX => self.data_port = Some(self.latched_address),
            command::GET_HEARTBEAT => {
                self.data_port = Some(self.heartbeat);
                self.heartbeat ^= 0xFF;
            }
            _ => {} // not understood: stays "busy", as on the real firmware
        }
    }

    /// The text in the buffer up to its first zero byte.
    fn buffer_string(&self) -> String {
        let end = self
            .buffer
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.buffer.len());
        self.buffer[..end].iter().map(|&b| b as char).collect()
    }

    /// Run `action` with the name in the buffer and reply "complete" with the
    /// error code it gives (0 for success).
    fn reply_complete_with(&mut self, action: impl FnOnce(&mut Card, &str) -> FsResult<()>) {
        let name = self.buffer_string();
        let code = match &mut self.card {
            Some(card) => action(card, &name).err().unwrap_or(error::NONE),
            None => error::INVALID_OBJECT,
        };
        self.data_port = Some(status::COMPLETE | code);
    }

    fn directory_open(&mut self) {
        let path = self.buffer_string();
        let result = self.card.as_mut().map(|card| card.open_directory(&path));
        self.data_port = Some(match result {
            Some(Ok(())) => status::OK,
            Some(Err(code)) => status::COMPLETE | code,
            None => status::COMPLETE | error::INVALID_OBJECT,
        });
    }

    /// Put the next matching entry in the buffer as: the name (a directory's
    /// in `<` `>`), a zero byte, the FAT attribute byte and the size (4 bytes).
    fn directory_read(&mut self) {
        let entry = self.card.as_mut().and_then(|card| card.read_directory());
        let Some(entry) = entry else {
            self.data_port = Some(status::COMPLETE);
            return;
        };
        let name = if entry.is_directory {
            format!("<{}>", entry.name)
        } else {
            entry.name.clone()
        };
        let name = name.as_bytes();
        self.buffer[..name.len()].copy_from_slice(name);
        self.buffer[name.len()] = 0;
        self.buffer[name.len() + 1] = if entry.is_directory {
            ATTRIBUTE_DIRECTORY
        } else {
            0
        };
        self.buffer[name.len() + 2..name.len() + 6].copy_from_slice(&entry.size.to_le_bytes());
        self.data_port = Some(status::OK);
    }

    fn rename(&mut self) {
        let from = self.buffer_string();
        let to: String = {
            let start = from.len() + 1;
            let rest = &self.buffer[start.min(255)..];
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            rest[..end].iter().map(|&b| b as char).collect()
        };
        let code = match &mut self.card {
            Some(card) => card.rename(&from, &to).err().unwrap_or(error::NONE),
            None => error::INVALID_OBJECT,
        };
        self.data_port = Some(status::COMPLETE | code);
    }

    fn file_open(&mut self, how: OpenHow) {
        let path = self.buffer_string();
        let requested = self.file_number;
        let Some(card) = &mut self.card else {
            self.data_port = Some(status::COMPLETE | error::INVALID_OBJECT);
            return;
        };
        let (slot, code) = card.open_file(requested, &path, how);
        self.file_number = slot;
        self.data_port = Some(status::COMPLETE | code);
    }

    /// Reply to the Atom with the size, start sector, position and attributes
    /// of the open file.
    fn file_get_info(&mut self) {
        let slot = self.file_number;
        let info = self.card.as_mut().and_then(|card| {
            let open = card.file(slot).ok()?;
            let size = open.file.metadata().ok()?.len() as u32;
            let position = open.file.stream_position().ok()? as u32;
            Some((size, position, open.attribute))
        });
        let Some((size, position, attribute)) = info else {
            self.data_port = Some(status::COMPLETE | error::INVALID_OBJECT);
            return;
        };
        self.buffer[0..4].copy_from_slice(&size.to_le_bytes());
        self.buffer[4..8].copy_from_slice(&0u32.to_le_bytes()); // start sector: not meaningful
        self.buffer[8..12].copy_from_slice(&position.to_le_bytes());
        self.buffer[12] = attribute & 0x3F;
        self.data_port = Some(status::OK);
    }

    fn file_read(&mut self) {
        let wanted = if self.amount == 0 {
            256
        } else {
            self.amount as usize
        };
        let slot = self.file_number;
        let Some(card) = &mut self.card else {
            self.data_port = Some(status::COMPLETE | error::INVALID_OBJECT);
            return;
        };
        let result = card.file(slot).and_then(|open| {
            let mut total = 0;
            while total < wanted {
                match open.file.read(&mut self.buffer[total..wanted]) {
                    Ok(0) => break,
                    Ok(n) => total += n,
                    Err(_) => return Err(error::DISK_ERROR),
                }
            }
            Ok(total)
        });
        self.data_port = Some(match result {
            // Random-access files report running out of file; the scratch
            // file (0) does not, the ROM works from its length.
            Ok(read) if slot > 0 && read != wanted => status::END_OF_FILE,
            Ok(_) => status::COMPLETE,
            Err(code) => status::COMPLETE | code,
        });
    }

    fn file_write(&mut self) {
        let amount = if self.amount == 0 {
            256
        } else {
            self.amount as usize
        };
        let slot = self.file_number;
        let Some(card) = &mut self.card else {
            self.data_port = Some(status::COMPLETE | error::INVALID_OBJECT);
            return;
        };
        let result = card.file(slot).and_then(|open| {
            open.file
                .write_all(&self.buffer[..amount])
                .map_err(io_error_code)
        });
        self.data_port = Some(status::COMPLETE | result.err().unwrap_or(error::NONE));
    }

    fn file_seek(&mut self) {
        let position = u32::from_le_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]);
        let slot = self.file_number;
        let result = match &mut self.card {
            Some(card) => card.file(slot).and_then(|open| {
                open.file
                    .seek(SeekFrom::Start(position as u64))
                    .map_err(io_error_code)
            }),
            None => Err(error::INVALID_OBJECT),
        };
        self.data_port = Some(status::COMPLETE | result.err().unwrap_or(error::NONE));
    }

    // ---- SDDOS: disk images as drives ---------------------------------------------

    /// Mount an image. The buffer holds the drive number, then the image's
    /// name. Replies with the image's attribute byte, or "complete" and an
    /// error (which also leaves the drive empty).
    fn image_mount(&mut self) {
        let drive = usize::from(self.buffer[0] & 3);
        let name: String = self.buffer[1..]
            .iter()
            .take_while(|&&b| b != 0)
            .take(sddos::NAME_LENGTH + 1)
            .map(|&b| b as char)
            .collect();
        let reply = match self.mount_image(drive, &name) {
            Ok(attributes) => attributes,
            Err(code) => status::COMPLETE | code,
        };
        self.save_image_list();
        self.data_port = Some(reply);
    }

    /// Find `name` on the card (in the current folder) and mount it.
    fn mount_image(&mut self, drive: usize, name: &str) -> Result<u8, u8> {
        let Some(card) = &self.card else {
            return Err(error::INVALID_OBJECT);
        };
        let (path, _) = card.resolve_file(name)?;
        self.drives.mount(drive, name, &path)
    }

    /// Choose the drive and sector the next read or write uses: the buffer
    /// holds the drive number, then the sector number (4 bytes, 256-byte
    /// sectors counted from the start of the image).
    fn image_load_position(&mut self) {
        let sector = u32::from_le_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]);
        self.drives
            .load_position(usize::from(self.buffer[0] & 3), sector);
    }

    /// The four image names, each ended by a zero (empty for an empty drive).
    fn image_names(&mut self) {
        let mut at = 0;
        for name in self.drives.names() {
            for byte in name.bytes() {
                self.buffer[at] = byte;
                at += 1;
            }
            self.buffer[at] = 0;
            at += 1;
        }
        self.data_port = Some(status::OK);
    }

    fn image_read_sector(&mut self) {
        self.data_port = Some(match self.drives.read_sector(&mut self.buffer) {
            Ok(()) => status::OK,
            Err(code) => status::COMPLETE | code,
        });
    }

    fn image_write_sector(&mut self) {
        self.data_port = Some(match self.drives.write_sector(&self.buffer) {
            Ok(()) => status::OK,
            Err(code) => status::COMPLETE | code,
        });
    }

    /// Remember the mounted images in `BOOTDRV.CFG` on the card.
    fn save_image_list(&self) {
        if let Some(card) = &self.card {
            // If the card cannot be written, the drives still work this time.
            let _ = fs::write(
                card.root.join(sddos::CONFIG_FILE),
                self.drives.config_bytes(),
            );
        }
    }

    /// Mount again whatever `BOOTDRV.CFG` says was mounted, dropping any
    /// image that has gone.
    fn image_restore_list(&mut self) {
        let saved = self
            .card
            .as_ref()
            .and_then(|card| fs::read(card.root.join(sddos::CONFIG_FILE)).ok())
            .unwrap_or_default();
        for (drive, name) in sddos::names_in_config(&saved).into_iter().enumerate() {
            self.drives.unmount(drive);
            if let Some(name) = name {
                // An image that cannot be found any more is simply dropped.
                let _ = self.mount_image(drive, &name);
            }
        }
        self.save_image_list();
        self.data_port = Some(status::OK);
    }

    /// "Execute packet": the only packets are read and write of the EEPROM.
    fn execute_packet(&mut self) {
        if self.amount == 0 && !self.data_present {
            self.data_port = Some(status::COMPLETE | error::NO_DATA);
            return;
        }
        let start = self.buffer[2] as usize;
        let count = self.buffer[3] as usize;
        match (self.buffer[0], self.buffer[1]) {
            (b'E', b'R') => {
                for n in 0..count {
                    self.buffer[n] = self.eeprom[(start + n) % EEPROM_SIZE];
                }
                self.data_port = Some(status::OK);
            }
            (b'E', b'W') => {
                for n in 0..count {
                    self.eeprom[(start + n) % EEPROM_SIZE] = self.buffer[(4 + n) % 256];
                }
                self.data_port = Some(status::OK);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    // ---- A tiny Atom-side driver for the registers ----------------------------

    const COMMAND: u16 = 0xB400;
    const LATCH: u16 = 0xB401;
    const READ_DATA: u16 = 0xB402;
    const WRITE_DATA: u16 = 0xB403;

    /// A scratch folder that is removed afterwards.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(label: &str) -> TempFolder {
            // Tests run in parallel, so each folder needs its own name.
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("atom-rs-{label}-{}-{unique}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn command(mmc: &mut AtoMmc, command: u8) -> u8 {
        mmc.write(COMMAND, command);
        mmc.read(COMMAND)
    }

    /// Send `text` to the buffer as a zero-terminated string.
    fn send_string(mmc: &mut AtoMmc, text: &str) {
        command(mmc, command::INIT_WRITE);
        for byte in text.bytes().chain([0]) {
            mmc.write(WRITE_DATA, byte);
        }
    }

    /// Read `count` bytes from the buffer.
    fn fetch(mmc: &mut AtoMmc, count: usize) -> Vec<u8> {
        // No status poll after INIT_READ: the first byte is ready to be read
        // from the data register straight away.
        mmc.write(COMMAND, command::INIT_READ);
        (0..count).map(|_| mmc.read(READ_DATA)).collect()
    }

    /// List a directory the way the ROM does; each line is the name as shown.
    fn list(mmc: &mut AtoMmc, path: &str) -> Vec<String> {
        send_string(mmc, path);
        assert_eq!(command(mmc, command::DIR_OPEN), status::OK);
        let mut names = Vec::new();
        while command(mmc, command::DIR_READ) == status::OK {
            let entry = fetch(mmc, 40);
            let end = entry.iter().position(|&b| b == 0).unwrap();
            names.push(String::from_utf8(entry[..end].to_vec()).unwrap());
        }
        names
    }

    fn open(mmc: &mut AtoMmc, command_byte: u8, name: &str) -> u8 {
        send_string(mmc, name);
        command(mmc, command_byte)
    }

    fn read_file(mmc: &mut AtoMmc, name: &str) -> Vec<u8> {
        assert_eq!(open(mmc, command::FILE_OPEN_READ, name), status::COMPLETE);
        let mut contents = Vec::new();
        loop {
            mmc.write(LATCH, 0); // 0 = 256 bytes
            let reply = command(mmc, command::READ_BYTES);
            let chunk = fetch(mmc, 256);
            contents.extend(chunk);
            if reply != status::COMPLETE {
                panic!("unexpected reply {reply:02X}");
            }
            // Stop when we have everything the file info says.
            send_nothing(mmc);
            let size = file_size(mmc);
            if contents.len() >= size {
                contents.truncate(size);
                break;
            }
        }
        command(mmc, command::FILE_CLOSE);
        contents
    }

    fn send_nothing(_: &mut AtoMmc) {}

    fn file_size(mmc: &mut AtoMmc) -> usize {
        assert_eq!(command(mmc, command::FILE_GET_INFO), status::OK);
        let info = fetch(mmc, 4);
        u32::from_le_bytes([info[0], info[1], info[2], info[3]]) as usize
    }

    // ---- Without a card ---------------------------------------------------------

    #[test]
    fn reads_as_open_bus_until_a_command_it_understands_is_given() {
        let mut mmc = AtoMmc::default();
        assert_eq!(mmc.read(0xB401), 0xB4);
        mmc.write(COMMAND, 0x20); // an SD card command: not emulated without a card
        assert_eq!(mmc.read(0xB401), 0xB4);
    }

    #[test]
    fn with_no_joystick_the_read_port_command_returns_zero() {
        // Games read this and treat 0 as "no joystick".
        let mut mmc = AtoMmc::default();
        mmc.write(COMMAND, 0xA2);
        assert_eq!(mmc.read(0xB401), 0);
    }

    #[test]
    fn read_port_returns_the_joystick_pins() {
        let mut mmc = AtoMmc {
            joystick: Some(0b1111_1101),
            ..AtoMmc::default()
        };
        mmc.write(COMMAND, 0xA2);
        assert_eq!(mmc.read(0xB401), 0b1111_1101);
    }

    #[test]
    fn port_direction_is_set_from_the_data_latch() {
        let mut mmc = AtoMmc::default();
        mmc.write(LATCH, 0xFF);
        mmc.write(COMMAND, 0xA1);
        assert_eq!(mmc.read(0xB401), 0x3F, "STATUS_OK");
        mmc.write(COMMAND, 0xA0);
        assert_eq!(mmc.read(0xB401), 0xFF, "the direction register reads back");
    }

    #[test]
    fn reset_clears_the_interface_but_keeps_the_joystick() {
        let mut mmc = AtoMmc {
            joystick: Some(5),
            ..AtoMmc::default()
        };
        mmc.write(COMMAND, 0xA2);
        mmc.reset();
        assert_eq!(mmc.read(0xB401), 0xB4);
        assert_eq!(mmc.joystick, Some(5));
    }

    // ---- Name handling ------------------------------------------------------------

    #[test]
    fn valid_names_are_8_dot_3() {
        assert_eq!(validate_name("MENU").unwrap(), "MENU");
        assert_eq!(validate_name("CCH-EGG").unwrap(), "CCH-EGG");
        assert_eq!(validate_name("NAME.TXT").unwrap(), "NAME.TXT");
        assert_eq!(
            validate_name("NAME.").unwrap(),
            "NAME",
            "a dangling dot is dropped"
        );
        assert_eq!(validate_name("TOOLONGNAME"), Err(error::INVALID_NAME));
        assert_eq!(validate_name("A.TOOL"), Err(error::INVALID_NAME));
        assert_eq!(validate_name("A.B.C"), Err(error::INVALID_NAME));
        assert_eq!(validate_name(""), Err(error::INVALID_NAME));
    }

    #[test]
    fn wildcards_match_like_the_firmware() {
        assert!(wildcard_matches("*", "ANYTHING"));
        assert!(wildcard_matches("A*", "ABC"));
        assert!(!wildcard_matches("A*", "BAC"));
        assert!(wildcard_matches("?BC", "ABC"));
        assert!(wildcard_matches("*.TXT", "NOTES.TXT"));
        assert!(wildcard_matches("a*", "ABC"), "case is ignored");
        assert!(!wildcard_matches("ABC", "ABCD"));
    }

    #[test]
    fn a_path_is_split_from_its_wildcard() {
        assert_eq!(
            split_wildcard("LIB/A*"),
            ("LIB".to_string(), "A*".to_string())
        );
        assert_eq!(
            split_wildcard("*.TXT"),
            (String::new(), "*.TXT".to_string())
        );
        assert_eq!(split_wildcard("LIB"), ("LIB".to_string(), "*".to_string()));
    }

    // ---- With a card -------------------------------------------------------------

    fn card_with_files() -> (TempFolder, AtoMmc) {
        let folder = TempFolder::new("card");
        fs::create_dir(folder.0.join("GAMES")).unwrap();
        fs::write(folder.0.join("MENU"), b"menu program").unwrap();
        fs::write(folder.0.join("GAMES").join("PANIC"), vec![7u8; 300]).unwrap();
        fs::write(folder.0.join("GAMES").join("FROGGER"), b"frog").unwrap();
        fs::write(folder.0.join("AVERYLONGNAME"), b"not shown").unwrap(); // not a valid 8.3 name
        let mmc = AtoMmc::with_card(folder.0.clone());
        (folder, mmc)
    }

    #[test]
    fn lists_the_card_with_directories_in_angle_brackets() {
        let (_folder, mut mmc) = card_with_files();
        let names = list(&mut mmc, "");
        assert_eq!(names, vec!["<GAMES>", "MENU"], "sorted, and 8.3 names only");
    }

    #[test]
    fn listing_can_use_a_path_and_a_wildcard() {
        let (_folder, mut mmc) = card_with_files();
        assert_eq!(list(&mut mmc, "GAMES"), vec!["FROGGER", "PANIC"]);
        assert_eq!(list(&mut mmc, "GAMES/P*"), vec!["PANIC"]);
        assert_eq!(
            list(&mut mmc, "/games/f*"),
            vec!["FROGGER"],
            "case is ignored"
        );
    }

    #[test]
    fn entries_carry_their_size() {
        let (_folder, mut mmc) = card_with_files();
        send_string(&mut mmc, "GAMES/PAN*");
        assert_eq!(command(&mut mmc, command::DIR_OPEN), status::OK);
        command(&mut mmc, command::DIR_READ);
        let entry = fetch(&mut mmc, 16);
        let name_end = entry.iter().position(|&b| b == 0).unwrap();
        assert_eq!(&entry[..name_end], b"PANIC");
        let size = u32::from_le_bytes(entry[name_end + 2..name_end + 6].try_into().unwrap());
        assert_eq!(size, 300);
    }

    #[test]
    fn changing_directory_changes_what_relative_names_mean() {
        let (_folder, mut mmc) = card_with_files();
        send_string(&mut mmc, "GAMES");
        assert_eq!(command(&mut mmc, command::DIR_CWD), status::COMPLETE);
        assert_eq!(list(&mut mmc, ""), vec!["FROGGER", "PANIC"]);
        send_string(&mut mmc, "..");
        assert_eq!(command(&mut mmc, command::DIR_CWD), status::COMPLETE);
        assert_eq!(list(&mut mmc, ""), vec!["<GAMES>", "MENU"]);
        send_string(&mut mmc, "NOSUCH");
        assert_eq!(
            command(&mut mmc, command::DIR_CWD),
            status::COMPLETE | error::NO_PATH
        );
    }

    #[test]
    fn nothing_outside_the_card_can_be_reached() {
        let (_folder, mut mmc) = card_with_files();
        send_string(&mut mmc, "../../../..");
        assert_eq!(
            command(&mut mmc, command::DIR_CWD),
            status::COMPLETE,
            "stays at the root"
        );
        assert_eq!(list(&mut mmc, ""), vec!["<GAMES>", "MENU"]);
        send_string(&mut mmc, "../../etc/passwd");
        assert_ne!(command(&mut mmc, command::FILE_OPEN_READ), status::COMPLETE);
    }

    #[test]
    fn reads_a_file_in_chunks() {
        let (_folder, mut mmc) = card_with_files();
        assert_eq!(read_file(&mut mmc, "games/panic"), vec![7u8; 300]);
        assert_eq!(read_file(&mut mmc, "MENU"), b"menu program");
    }

    #[test]
    fn opening_a_missing_file_reports_not_found() {
        let (_folder, mut mmc) = card_with_files();
        assert_eq!(
            open(&mut mmc, command::FILE_OPEN_READ, "NOPE"),
            status::COMPLETE | error::NO_FILE
        );
        assert_eq!(
            open(&mut mmc, command::FILE_OPEN_READ, "NODIR/FILE"),
            status::COMPLETE | error::NO_PATH
        );
        assert_eq!(
            open(&mut mmc, command::FILE_OPEN_READ, "WAYTOOLONGNAME"),
            status::COMPLETE | error::INVALID_NAME
        );
    }

    #[test]
    fn writes_a_new_file_and_refuses_to_overwrite_one() {
        let (folder, mut mmc) = card_with_files();
        assert_eq!(
            open(&mut mmc, command::FILE_OPEN_WRITE, "SAVED"),
            status::COMPLETE
        );
        send_string(&mut mmc, ""); // fill the buffer with the data to write
        command(&mut mmc, command::INIT_WRITE);
        for byte in 0..10u8 {
            mmc.write(WRITE_DATA, byte + 100);
        }
        mmc.write(LATCH, 10);
        assert_eq!(command(&mut mmc, command::WRITE_BYTES), status::COMPLETE);
        command(&mut mmc, command::FILE_CLOSE);
        assert_eq!(
            fs::read(folder.0.join("SAVED")).unwrap(),
            (100..110u8).collect::<Vec<_>>()
        );

        assert_eq!(
            open(&mut mmc, command::FILE_OPEN_WRITE, "SAVED"),
            status::COMPLETE | error::EXISTS
        );
    }

    #[test]
    fn deletes_renames_and_makes_directories() {
        let (folder, mut mmc) = card_with_files();

        send_string(&mut mmc, "NEWDIR");
        assert_eq!(command(&mut mmc, command::DIR_MKDIR), status::COMPLETE);
        assert!(folder.0.join("NEWDIR").is_dir());
        send_string(&mut mmc, "NEWDIR");
        assert_eq!(
            command(&mut mmc, command::DIR_MKDIR),
            status::COMPLETE | error::EXISTS
        );

        // Rename: the new name follows the old one in the buffer.
        command(&mut mmc, command::INIT_WRITE);
        for byte in b"MENU\0START\0" {
            mmc.write(WRITE_DATA, *byte);
        }
        assert_eq!(command(&mut mmc, command::RENAME), status::COMPLETE);
        assert!(folder.0.join("START").exists() && !folder.0.join("MENU").exists());

        assert_eq!(
            open(&mut mmc, command::FILE_DELETE, "START"),
            status::COMPLETE
        );
        assert!(!folder.0.join("START").exists());
        assert_eq!(
            open(&mut mmc, command::FILE_DELETE, "START"),
            status::COMPLETE | error::NO_PATH
        );

        send_string(&mut mmc, "NEWDIR");
        assert_eq!(command(&mut mmc, command::DIR_RMDIR), status::COMPLETE);
        assert!(!folder.0.join("NEWDIR").exists());
    }

    #[test]
    fn random_access_files_get_numbers_and_report_end_of_file() {
        let (_folder, mut mmc) = card_with_files();
        // Open with file number 1 (bits 5-6 of the command).
        send_string(&mut mmc, "GAMES/FROGGER");
        let reply = command(&mut mmc, command::FILE_OPEN_READ | (1 << 5));
        assert_eq!(reply, status::COMPLETE | FILE_NUMBER_OFFSET | 1);
        // Ask for 10 bytes of a 4 byte file: end of file.
        mmc.write(LATCH, 10);
        let reply = command(&mut mmc, command::READ_BYTES | (1 << 2));
        assert_eq!(reply, status::END_OF_FILE);
        assert_eq!(&fetch(&mut mmc, 4), b"frog");
    }

    #[test]
    fn a_fifth_open_file_is_refused() {
        let (_folder, mut mmc) = card_with_files();
        // Slots 1, 2 and 3 are the random access ones.
        for n in 1..=3u8 {
            send_string(&mut mmc, "MENU");
            let reply = command(&mut mmc, command::FILE_OPEN_READ | (1 << 5));
            assert_eq!(reply, status::COMPLETE | FILE_NUMBER_OFFSET | n);
        }
        send_string(&mut mmc, "MENU");
        let reply = command(&mut mmc, command::FILE_OPEN_READ | (1 << 5));
        assert_eq!(reply, status::COMPLETE | error::TOO_MANY_OPEN);
    }

    #[test]
    fn seek_and_info_work_on_an_open_file() {
        let (_folder, mut mmc) = card_with_files();
        assert_eq!(
            open(&mut mmc, command::FILE_OPEN_READ, "GAMES/PANIC"),
            status::COMPLETE
        );
        assert_eq!(file_size(&mut mmc), 300);
        command(&mut mmc, command::INIT_WRITE);
        for byte in 290u32.to_le_bytes() {
            mmc.write(WRITE_DATA, byte);
        }
        assert_eq!(command(&mut mmc, command::FILE_SEEK), status::COMPLETE);
        mmc.write(LATCH, 20);
        // File 0 does not report end of file, but only 10 bytes are left.
        assert_eq!(command(&mut mmc, command::READ_BYTES), status::COMPLETE);
    }

    #[test]
    fn reset_goes_back_to_the_root_and_closes_files() {
        let (_folder, mut mmc) = card_with_files();
        send_string(&mut mmc, "GAMES");
        command(&mut mmc, command::DIR_CWD);
        mmc.reset();
        assert_eq!(list(&mut mmc, ""), vec!["<GAMES>", "MENU"]);
    }

    #[test]
    fn utility_commands_answer_with_a_card() {
        let (_folder, mut mmc) = card_with_files();
        assert_eq!(command(&mut mmc, command::GET_CARD_TYPE), 1);
        assert_eq!(command(&mut mmc, command::GET_FIRMWARE_VERSION), 0x2D);
        assert_eq!(command(&mut mmc, command::GET_HEARTBEAT), 0x55);
        assert_eq!(command(&mut mmc, command::GET_HEARTBEAT), 0xAA);
        mmc.write(LATCH, 0x5A);
        assert_eq!(command(&mut mmc, command::SET_CONFIG_BYTE), status::OK);
        assert_eq!(command(&mut mmc, command::GET_CONFIG_BYTE), 0x5A);
    }

    // ---- SDDOS: disk images as drives ------------------------------------------------

    /// Send raw bytes to the buffer, as the ROM does before a command.
    fn send_bytes(mmc: &mut AtoMmc, bytes: &[u8]) {
        command(mmc, command::INIT_WRITE);
        for &byte in bytes {
            mmc.write(WRITE_DATA, byte);
        }
    }

    /// A card holding a 4-sector image in which sector `s` is full of `s + 1`.
    fn card_with_an_image() -> (TempFolder, AtoMmc) {
        let folder = TempFolder::new("sddos");
        let data: Vec<u8> = (0..4).flat_map(|s| vec![s as u8 + 1; 256]).collect();
        fs::write(folder.0.join("DISK.SSD"), data).unwrap();
        let mmc = AtoMmc::with_card(folder.0.clone());
        (folder, mmc)
    }

    fn mount(mmc: &mut AtoMmc, drive: u8, name: &str) -> u8 {
        let mut bytes = vec![drive];
        bytes.extend(name.bytes());
        bytes.push(0);
        send_bytes(mmc, &bytes);
        command(mmc, command::FILE_OPEN_IMAGE)
    }

    fn read_image_sector(mmc: &mut AtoMmc, drive: u8, sector: u32) -> (u8, Vec<u8>) {
        let mut position = vec![drive];
        position.extend(sector.to_le_bytes());
        send_bytes(mmc, &position);
        mmc.write(COMMAND, command::LOAD_POSITION);
        let reply = command(mmc, command::READ_IMAGE_SECTOR);
        (reply, fetch(mmc, 256))
    }

    #[test]
    fn mounting_an_image_replies_with_its_attributes() {
        let (_folder, mut mmc) = card_with_an_image();
        assert_eq!(
            mount(&mut mmc, 0, "disk.ssd"),
            0x20,
            "found, whatever the case"
        );
        mmc.write(LATCH, 0);
        assert_eq!(command(&mut mmc, command::GET_IMAGE_STATUS), 0x20);
        mmc.write(LATCH, 1);
        assert_eq!(
            command(&mut mmc, command::GET_IMAGE_STATUS),
            0xFF,
            "drive 1 is empty"
        );
    }

    #[test]
    fn mounting_a_missing_or_badly_named_image_fails_and_leaves_the_drive_empty() {
        let (_folder, mut mmc) = card_with_an_image();
        assert_eq!(mount(&mut mmc, 0, "NOPE.SSD"), status::COMPLETE | 0x04);
        assert_eq!(
            mount(&mut mmc, 0, "WAYTOOLONGNAME.SSD"),
            status::COMPLETE | 0x06
        );
        mmc.write(LATCH, 0);
        assert_eq!(command(&mut mmc, command::GET_IMAGE_STATUS), 0xFF);
    }

    #[test]
    fn the_same_image_cannot_be_in_two_drives() {
        let (_folder, mut mmc) = card_with_an_image();
        assert_eq!(mount(&mut mmc, 0, "DISK.SSD"), 0x20);
        assert_eq!(mount(&mut mmc, 1, "DISK.SSD"), status::COMPLETE | 0x0A);
    }

    #[test]
    fn reads_sectors_of_a_mounted_image() {
        let (_folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 2, "DISK.SSD");
        let (reply, data) = read_image_sector(&mut mmc, 2, 0);
        assert_eq!(reply, status::OK);
        assert_eq!(data, vec![1; 256]);
        let (reply, data) = read_image_sector(&mut mmc, 2, 3);
        assert_eq!((reply, data[0], data[255]), (status::OK, 4, 4));
    }

    #[test]
    fn reading_an_empty_drive_is_an_error() {
        let (_folder, mut mmc) = card_with_an_image();
        let (reply, _) = read_image_sector(&mut mmc, 3, 0);
        assert_eq!(reply, status::COMPLETE | 0x09);
    }

    #[test]
    fn writes_a_sector_into_the_image_file() {
        let (folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 0, "DISK.SSD");
        let mut position = vec![0];
        position.extend(1u32.to_le_bytes());
        send_bytes(&mut mmc, &position);
        mmc.write(COMMAND, command::LOAD_POSITION);
        send_bytes(&mut mmc, &[0xAB; 256]);
        assert_eq!(command(&mut mmc, command::WRITE_IMAGE_SECTOR), status::OK);
        let file = fs::read(folder.0.join("DISK.SSD")).unwrap();
        assert_eq!(&file[256..512], &[0xAB; 256]);
        assert_eq!(&file[0..256], &[1; 256], "its neighbours are untouched");
    }

    #[test]
    fn setting_the_position_gets_no_reply() {
        // The ROM does not wait for one, and the real firmware gives none.
        let (_folder, mut mmc) = card_with_an_image();
        send_bytes(&mut mmc, &[0, 0, 0, 0, 0]);
        mmc.write(COMMAND, command::LOAD_POSITION);
        assert_eq!(mmc.read(COMMAND), status::BUSY);
    }

    #[test]
    fn lists_the_image_names_with_empty_drives_as_empty_strings() {
        let (_folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 1, "DISK.SSD");
        assert_eq!(command(&mut mmc, command::GET_IMAGE_NAMES), status::OK);
        assert_eq!(fetch(&mut mmc, 12), b"\0DISK.SSD\0\0\0");
    }

    #[test]
    fn unmounting_empties_the_drive() {
        let (_folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 0, "DISK.SSD");
        mmc.write(LATCH, 0);
        assert_eq!(command(&mut mmc, command::UNMOUNT_IMAGE), status::OK);
        mmc.write(LATCH, 0);
        assert_eq!(command(&mut mmc, command::GET_IMAGE_STATUS), 0xFF);
    }

    #[test]
    fn mounted_images_are_remembered_in_bootdrv_cfg_and_come_back() {
        let (folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 2, "DISK.SSD");
        let config = fs::read(folder.0.join("BOOTDRV.CFG")).unwrap();
        assert_eq!(config.len(), 56);
        assert_eq!(&config[28..35], b"DISK.SSD".get(..7).unwrap());

        // A fresh card interface (a new power-on) knows nothing...
        let mut again = AtoMmc::with_card(folder.0.clone());
        again.write(LATCH, 2);
        assert_eq!(command(&mut again, command::GET_IMAGE_STATUS), 0xFF);
        // ...until the ROM asks it to restore the list.
        assert_eq!(command(&mut again, command::RESTORE_IMAGE_LIST), status::OK);
        again.write(LATCH, 2);
        assert_eq!(command(&mut again, command::GET_IMAGE_STATUS), 0x20);
        assert_eq!(read_image_sector(&mut again, 2, 1).1, vec![2; 256]);
    }

    #[test]
    fn an_image_that_has_gone_is_dropped_when_the_list_is_restored() {
        let (folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 0, "DISK.SSD");
        fs::remove_file(folder.0.join("DISK.SSD")).unwrap();
        let mut again = AtoMmc::with_card(folder.0.clone());
        assert_eq!(command(&mut again, command::RESTORE_IMAGE_LIST), status::OK);
        again.write(LATCH, 0);
        assert_eq!(command(&mut again, command::GET_IMAGE_STATUS), 0xFF);
    }

    #[test]
    fn mounted_drives_survive_a_reset_of_the_atom() {
        let (_folder, mut mmc) = card_with_an_image();
        mount(&mut mmc, 0, "DISK.SSD");
        mmc.reset();
        mmc.write(LATCH, 0);
        assert_eq!(command(&mut mmc, command::GET_IMAGE_STATUS), 0x20);
    }

    #[test]
    fn unknown_commands_stay_busy() {
        let (_folder, mut mmc) = card_with_files();
        assert_eq!(command(&mut mmc, 0x48), status::BUSY);
    }
}
