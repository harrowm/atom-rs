//! The Intel 8271 floppy disk controller, as fitted to the Atom's disk
//! interface at `0A00`.
//!
//! ## How the CPU talks to it
//!
//! | Address | Read | Write |
//! |---------|------|-------|
//! | `0A00` | status | command (and drive select) |
//! | `0A01` | result | parameter |
//! | `0A02` | | reset (bit 0) |
//! | `0A04` | data | data |
//!
//! The CPU writes a command, then its parameters (how many depends on the
//! command). The chip then does the work and either gives a *result* (a one
//! byte completion code) or, for sector reads and writes, moves the data a
//! byte at a time, raising an NMI for each byte. The CPU's NMI handler
//! reads or writes the data register. After the last byte, another NMI says
//! "done" and the result register holds 0 if all went well.
//!
//! ## What is simplified
//!
//! Disk rotation is not modelled: a sector starts a fixed time after the head
//! arrives, and bytes then flow at the real single-density rate of one every
//! 64 cycles. Seeks take a fixed time per track. CRC errors cannot happen.

use serde::{Deserialize, Serialize};

use super::disk::{DiskImage, SECTOR_SIZE, SECTORS_PER_TRACK};

/// One byte every 64 us: single density (FM) at 125 kbit/s.
const BYTE_CYCLES: u32 = 64;
const COMMAND_START_CYCLES: u32 = 1_000;
const SEEK_CYCLES_PER_TRACK: u32 = 3_000;
const SECTOR_START_CYCLES: u32 = 800;
const SECTOR_END_CYCLES: u32 = 200;

/// Status register bits.
mod status {
    pub const BUSY: u8 = 0x80;
    pub const RESULT_READY: u8 = 0x10;
    pub const INTERRUPT: u8 = 0x08;
    pub const DATA_REQUEST: u8 = 0x04;
    /// Busy, with an interrupt and a data request: "give me / take a byte".
    pub const BYTE_READY: u8 = BUSY | INTERRUPT | DATA_REQUEST;
    /// Command finished: result ready and interrupt (the "done" NMI).
    pub const DONE: u8 = RESULT_READY | INTERRUPT;
}

/// Result register codes.
mod result {
    pub const OK: u8 = 0x00;
    pub const DRIVE_NOT_READY: u8 = 0x10;
    pub const WRITE_PROTECTED: u8 = 0x12;
    pub const SECTOR_NOT_FOUND: u8 = 0x18;
}

/// Commands this controller understands (the low 6 bits of the command byte).
mod command {
    pub const WRITE_DATA: u8 = 0x0B;
    pub const READ_DATA: u8 = 0x13;
    pub const READ_ID: u8 = 0x1B;
    pub const VERIFY: u8 = 0x1F;
    pub const FORMAT: u8 = 0x23;
    pub const SEEK: u8 = 0x29;
    pub const READ_DRIVE_STATUS: u8 = 0x2C;
    pub const SPECIFY: u8 = 0x35;
    pub const WRITE_SPECIAL: u8 = 0x3A;
    pub const READ_SPECIAL: u8 = 0x3D;
}

/// Special registers reached through the "write/read special register" commands.
mod special {
    pub const SCAN_COUNT: u8 = 0x06;
    pub const DRIVE0_TRACK: u8 = 0x12;
    pub const MODE: u8 = 0x17;
    pub const DRIVE1_TRACK: u8 = 0x1A;
    /// Drive control output; bit 5 selects the disk side.
    pub const DRIVE_OUTPUT: u8 = 0x23;
}

/// How many parameter bytes each command takes.
fn parameters_needed(command: u8) -> usize {
    match command {
        command::SPECIFY => 4,
        command::FORMAT => 5,
        command::WRITE_SPECIAL => 2,
        command::SEEK | command::READ_SPECIAL => 1,
        command::READ_DATA | command::WRITE_DATA | command::READ_ID | command::VERIFY => 3,
        _ => 0,
    }
}

/// What the controller is waiting to do next (when `countdown` runs out).
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Task {
    Idle,
    /// The head is moving to the track in parameter 0.
    Seeking,
    /// Bytes of a sector (or ID, or format table) are passing; one per
    /// `BYTE_CYCLES`. For reads `buffer` is the data being delivered; for
    /// writes and formats it fills up with what the CPU supplies.
    Transfer {
        buffer: Vec<u8>,
        index: usize,
    },
    /// A sector is done; go on to the next, or finish.
    SectorEnd,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fdc8271 {
    status: u8,
    result: u8,
    data: u8,
    /// Has the CPU written the data register since we last took it?
    data_written: bool,

    command: u8,
    params: [u8; 5],
    param_count: usize,
    params_needed: usize,
    /// Which drive the command selected (0 or 1).
    drive: usize,
    /// Drive-select bits from the command byte (bit 0 = drive 0, bit 1 = drive 1).
    drive_select: u8,
    /// Where the controller believes each drive's head is.
    track: [u8; 2],
    drive_output: u8,

    sector: u8,
    sectors_left: u8,
    task: Task,
    countdown: u32,

    pub drives: [Option<DiskImage>; 2],
}

impl Fdc8271 {
    pub fn new() -> Self {
        Fdc8271 {
            status: 0,
            result: 0,
            data: 0,
            data_written: false,
            command: 0xFF,
            params: [0; 5],
            param_count: 0,
            params_needed: 0,
            drive: 0,
            drive_select: 0,
            track: [0; 2],
            drive_output: 0,
            sector: 0,
            sectors_left: 0,
            task: Task::Idle,
            countdown: 0,
            drives: [None, None],
        }
    }

    fn reset(&mut self) {
        let drives = std::mem::take(&mut self.drives);
        *self = Fdc8271::new();
        self.drives = drives;
    }

    /// The NMI output: asserted while the interrupt bit of the status is set.
    pub fn nmi_asserted(&self) -> bool {
        self.status & status::INTERRUPT != 0
    }

    /// Advance one CPU cycle.
    pub fn tick(&mut self) {
        if self.countdown == 0 {
            return;
        }
        self.countdown -= 1;
        if self.countdown == 0 {
            self.advance();
        }
    }

    // ---- Registers ----------------------------------------------------------

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr & 7 {
            0 => self.status,
            1 => {
                self.status &= !(status::RESULT_READY | status::INTERRUPT);
                std::mem::take(&mut self.result)
            }
            4 => {
                self.status &= !(status::INTERRUPT | status::DATA_REQUEST);
                self.data
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 7 {
            0 => self.start_command(value),
            1 => self.add_parameter(value),
            2 => {
                if value & 1 != 0 {
                    self.reset();
                }
            }
            4 => {
                self.data = value;
                self.data_written = true;
                self.status &= !(status::INTERRUPT | status::DATA_REQUEST);
            }
            _ => {}
        }
    }

    fn start_command(&mut self, value: u8) {
        if self.status & status::BUSY != 0 {
            return; // ignored while a command is running
        }
        self.command = match value & 0x3F {
            0x17 => command::READ_DATA, // "read data and deleted data"
            other => other,
        };
        self.drive_select = value >> 6;
        self.drive = if value & 0x80 != 0 { 1 } else { 0 };
        self.param_count = 0;
        self.params_needed = parameters_needed(self.command);
        self.status = status::BUSY;

        if self.params_needed == 0 {
            if self.command == command::READ_DRIVE_STATUS {
                let track0 = if self.track[self.drive] == 0 { 0x02 } else { 0 };
                let ready = |bit: u8, drive: usize| {
                    if self.drive_select & bit != 0 && self.drives[drive].is_some() {
                        bit_for(drive)
                    } else {
                        0
                    }
                };
                self.result = 0x80 | 0x08 | track0 | ready(1, 0) | ready(2, 1);
                self.status = status::RESULT_READY;
            } else {
                self.fail_with(result::DRIVE_NOT_READY, status::RESULT_READY);
            }
        }
    }

    fn add_parameter(&mut self, value: u8) {
        if self.param_count < self.params.len() {
            self.params[self.param_count] = value;
            self.param_count += 1;
        }
        if self.param_count == self.params_needed {
            self.run_command();
        }
    }

    // ---- Commands -----------------------------------------------------------

    fn run_command(&mut self) {
        let p = self.params;
        match self.command {
            command::SPECIFY => self.status = 0,
            command::WRITE_SPECIAL => {
                self.status = 0;
                match p[0] {
                    special::DRIVE0_TRACK => self.track[0] = p[1],
                    special::DRIVE1_TRACK => self.track[1] = p[1],
                    special::DRIVE_OUTPUT => self.drive_output = p[1],
                    special::MODE => {}
                    _ => self.fail_with(result::SECTOR_NOT_FOUND, status::DONE),
                }
            }
            command::READ_SPECIAL => {
                self.status = status::RESULT_READY;
                self.result = match p[0] {
                    special::SCAN_COUNT => 0,
                    special::DRIVE0_TRACK => self.track[0],
                    special::DRIVE1_TRACK => self.track[1],
                    special::DRIVE_OUTPUT => self.drive_output,
                    _ => {
                        self.fail_with(result::SECTOR_NOT_FOUND, status::DONE);
                        return;
                    }
                };
            }
            command::SEEK => self.begin_seek(),
            command::READ_DATA
            | command::WRITE_DATA
            | command::VERIFY
            | command::READ_ID
            | command::FORMAT => {
                if self.drives[self.drive].is_none() {
                    self.fail_with(result::DRIVE_NOT_READY, status::DONE);
                    return;
                }
                self.sector = p[1];
                self.sectors_left = (p[2] & 31).max(1);
                self.begin_seek();
            }
            _ => self.fail_with(result::SECTOR_NOT_FOUND, status::DONE),
        }
    }

    fn begin_seek(&mut self) {
        let distance = self.params[0].abs_diff(self.track[self.drive]) as u32;
        self.task = Task::Seeking;
        self.countdown = COMMAND_START_CYCLES + distance * SEEK_CYCLES_PER_TRACK;
    }

    /// The side of the disk the drive is set to read (bit 5 of the drive output).
    fn side(&self) -> u8 {
        (self.drive_output >> 5) & 1
    }

    fn finish_ok(&mut self) {
        self.fail_with(result::OK, status::DONE);
    }

    /// End the command with a result code (0 means success).
    fn fail_with(&mut self, code: u8, new_status: u8) {
        self.result = code;
        self.status = new_status;
        self.task = Task::Idle;
        self.countdown = 0;
    }

    // ---- The state machine ----------------------------------------------------

    fn advance(&mut self) {
        match std::mem::replace(&mut self.task, Task::Idle) {
            Task::Idle => {}
            Task::Seeking => {
                self.track[self.drive] = self.params[0];
                if self.command == command::SEEK {
                    self.finish_ok();
                } else {
                    self.begin_sector();
                }
            }
            Task::Transfer { buffer, index } => self.transfer_byte(buffer, index),
            Task::SectorEnd => {
                self.sectors_left -= 1;
                if self.sectors_left == 0 {
                    self.finish_ok();
                } else {
                    self.sector += 1;
                    self.begin_sector();
                }
            }
        }
    }

    /// Start working on `self.sector` of the current track.
    fn begin_sector(&mut self) {
        let (track, side, sector) = (self.params[0], self.side(), self.sector);
        let Some(disk) = &self.drives[self.drive] else {
            return self.fail_with(result::DRIVE_NOT_READY, status::DONE);
        };

        let buffer = match self.command {
            command::READ_DATA | command::VERIFY => match disk.read_sector(track, side, sector) {
                Some(bytes) => bytes.to_vec(),
                None => return self.fail_with(result::SECTOR_NOT_FOUND, status::DONE),
            },
            command::READ_ID => {
                if disk.read_sector(track, side, sector).is_none() {
                    return self.fail_with(result::SECTOR_NOT_FOUND, status::DONE);
                }
                // The sector's ID field: track, side, sector, size code (1 = 256 bytes).
                vec![track, side, sector, 1]
            }
            command::WRITE_DATA => {
                if disk.write_protected {
                    return self.fail_with(result::WRITE_PROTECTED, status::DONE);
                }
                self.request_byte();
                vec![0; SECTOR_SIZE]
            }
            command::FORMAT => {
                if disk.write_protected {
                    return self.fail_with(result::WRITE_PROTECTED, status::DONE);
                }
                // The CPU supplies the 4-byte ID of every sector on the track.
                self.request_byte();
                vec![0; 4 * self.sectors_left as usize]
            }
            _ => return self.fail_with(result::SECTOR_NOT_FOUND, status::DONE),
        };
        self.task = Task::Transfer { buffer, index: 0 };
        self.countdown = SECTOR_START_CYCLES;
    }

    /// One byte time has passed.
    fn transfer_byte(&mut self, mut buffer: Vec<u8>, mut index: usize) {
        let reading = matches!(
            self.command,
            command::READ_DATA | command::VERIFY | command::READ_ID
        );
        if reading {
            self.data = buffer[index];
            // Verify checks the data is readable but does not hand it over.
            if self.command != command::VERIFY {
                self.status = status::BYTE_READY;
            }
            self.result = 0;
        } else {
            // Take the byte the CPU supplied (or repeat the last one if it was late).
            buffer[index] = self.data;
            self.data_written = false;
        }
        index += 1;

        if index < buffer.len() {
            if !reading {
                self.request_byte();
            }
            self.task = Task::Transfer { buffer, index };
            self.countdown = BYTE_CYCLES;
            return;
        }

        // The last byte of this sector.
        match self.command {
            command::WRITE_DATA => {
                let (track, side, sector) = (self.params[0], self.side(), self.sector);
                if let Some(disk) = &mut self.drives[self.drive] {
                    disk.write_sector(track, side, sector, &buffer);
                }
            }
            command::FORMAT => {
                let (track, side) = (self.params[0], self.side());
                if let Some(disk) = &mut self.drives[self.drive] {
                    disk.format_track(track, side, 0xE5);
                }
                self.sectors_left = 1; // the whole track is done in one go
            }
            _ => {}
        }
        self.task = Task::SectorEnd;
        self.countdown = SECTOR_END_CYCLES;
    }

    /// Ask the CPU (by NMI) to supply the next byte.
    fn request_byte(&mut self) {
        self.status = status::BYTE_READY;
        self.result = 0;
    }

    /// Is a command running? (Used by the debugger display.)
    pub fn is_busy(&self) -> bool {
        self.status & status::BUSY != 0
    }

    pub fn status_byte(&self) -> u8 {
        self.status
    }

    pub fn current_track(&self, drive: usize) -> u8 {
        self.track[drive]
    }
}

/// The ready bit in the drive status result for drive 0 (0x04) or 1 (0x40).
fn bit_for(drive: usize) -> u8 {
    if drive == 0 { 0x04 } else { 0x40 }
}

impl Default for Fdc8271 {
    fn default() -> Self {
        Self::new()
    }
}

/// Sectors per track, re-exported for the front end.
pub const SECTORS: usize = SECTORS_PER_TRACK;

#[cfg(test)]
mod tests {
    use super::*;

    /// A controller with a blank single-sided disk in drive 0.
    fn fdc_with_disk() -> Fdc8271 {
        let mut fdc = Fdc8271::new();
        fdc.drives[0] = Some(DiskImage::blank_80_track(1));
        fdc
    }

    /// Run the controller until `condition` holds (panics if it never does).
    fn run_until(fdc: &mut Fdc8271, condition: impl Fn(&Fdc8271) -> bool) {
        for _ in 0..2_000_000 {
            if condition(fdc) {
                return;
            }
            fdc.tick();
        }
        panic!(
            "controller never reached the expected state (status {:02X})",
            fdc.status
        );
    }

    fn command(fdc: &mut Fdc8271, command: u8, params: &[u8]) {
        fdc.write(0, command | 0x40); // select drive 0
        for &p in params {
            fdc.write(1, p);
        }
    }

    /// Read a whole sector the way the DOS NMI handler would: one byte per NMI.
    fn read_sector_bytes(fdc: &mut Fdc8271, track: u8, sector: u8, count: u8) -> Vec<u8> {
        command(fdc, command::READ_DATA, &[track, sector, count]);
        let mut bytes = Vec::new();
        loop {
            run_until(fdc, |f| f.nmi_asserted());
            if fdc.status_byte() & status::DATA_REQUEST != 0 {
                bytes.push(fdc.read(4));
            } else {
                return bytes; // the "done" interrupt
            }
        }
    }

    #[test]
    fn reads_a_sector_one_byte_per_nmi() {
        let mut fdc = fdc_with_disk();
        let pattern: Vec<u8> = (0..=255).collect();
        fdc.drives[0]
            .as_mut()
            .unwrap()
            .write_sector(3, 0, 7, &pattern);

        let bytes = read_sector_bytes(&mut fdc, 3, 7, 1);
        assert_eq!(bytes, pattern);
        assert_eq!(fdc.read(1), result::OK);
        assert!(
            !fdc.nmi_asserted(),
            "reading the result clears the interrupt"
        );
        assert_eq!(fdc.current_track(0), 3);
    }

    #[test]
    fn reads_several_sectors_in_sequence() {
        let mut fdc = fdc_with_disk();
        fdc.drives[0]
            .as_mut()
            .unwrap()
            .write_sector(0, 0, 2, &[0xAA; SECTOR_SIZE]);
        fdc.drives[0]
            .as_mut()
            .unwrap()
            .write_sector(0, 0, 3, &[0xBB; SECTOR_SIZE]);
        let bytes = read_sector_bytes(&mut fdc, 0, 2, 2);
        assert_eq!(bytes.len(), 512);
        assert!(bytes[..256].iter().all(|&b| b == 0xAA));
        assert!(bytes[256..].iter().all(|&b| b == 0xBB));
    }

    #[test]
    fn writes_a_sector_taking_one_byte_per_nmi() {
        let mut fdc = fdc_with_disk();
        command(&mut fdc, command::WRITE_DATA, &[5, 1, 1]);
        for i in 0..SECTOR_SIZE {
            run_until(&mut fdc, |f| f.nmi_asserted());
            fdc.write(4, (i as u8).wrapping_mul(3));
        }
        run_until(&mut fdc, |f| f.status_byte() == status::DONE);
        assert_eq!(fdc.read(1), result::OK);

        let sector = fdc.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(5, 0, 1)
            .unwrap();
        assert_eq!(sector[10], 30);
        assert_eq!(sector[255], 255u8.wrapping_mul(3));
        assert!(fdc.drives[0].as_ref().unwrap().modified);
    }

    #[test]
    fn write_protected_disks_refuse_writes() {
        let mut fdc = fdc_with_disk();
        fdc.drives[0].as_mut().unwrap().write_protected = true;
        command(&mut fdc, command::WRITE_DATA, &[0, 0, 1]);
        run_until(&mut fdc, |f| f.status_byte() == status::DONE);
        assert_eq!(fdc.read(1), result::WRITE_PROTECTED);
    }

    #[test]
    fn missing_sector_and_missing_disk_report_errors() {
        let mut fdc = fdc_with_disk();
        command(&mut fdc, command::READ_DATA, &[90, 0, 1]); // track 90: off the end
        run_until(&mut fdc, |f| f.status_byte() == status::DONE);
        assert_eq!(fdc.read(1), result::SECTOR_NOT_FOUND);

        fdc.drives[0] = None;
        command(&mut fdc, command::READ_DATA, &[0, 0, 1]);
        assert_eq!(fdc.read(1), result::DRIVE_NOT_READY);
    }

    #[test]
    fn seek_moves_the_head_and_interrupts_when_done() {
        let mut fdc = fdc_with_disk();
        command(&mut fdc, command::SEEK, &[20]);
        assert!(fdc.is_busy());
        run_until(&mut fdc, |f| f.nmi_asserted());
        assert_eq!(fdc.read(1), result::OK);
        assert_eq!(fdc.current_track(0), 20);
        assert!(!fdc.is_busy());
    }

    #[test]
    fn drive_status_reports_ready_and_track_zero() {
        let mut fdc = fdc_with_disk();
        command(&mut fdc, command::READ_DRIVE_STATUS, &[]);
        let status_bits = fdc.read(1);
        assert_eq!(status_bits & 0x04, 0x04, "drive 0 ready");
        assert_eq!(status_bits & 0x02, 0x02, "at track 0");
        assert_eq!(status_bits & 0x40, 0, "drive 1 not selected or empty");
    }

    #[test]
    fn special_registers_read_back() {
        let mut fdc = fdc_with_disk();
        command(
            &mut fdc,
            command::WRITE_SPECIAL,
            &[special::DRIVE0_TRACK, 17],
        );
        command(&mut fdc, command::READ_SPECIAL, &[special::DRIVE0_TRACK]);
        assert_eq!(fdc.read(1), 17);
    }

    #[test]
    fn formatting_fills_the_track() {
        let mut fdc = fdc_with_disk();
        fdc.drives[0]
            .as_mut()
            .unwrap()
            .write_sector(4, 0, 0, &[1; SECTOR_SIZE]);
        // Format track 4: 10 sectors, the CPU supplies 4 ID bytes for each.
        command(&mut fdc, command::FORMAT, &[4, 0, 10, 0, 0]);
        for _ in 0..40 {
            run_until(&mut fdc, |f| f.nmi_asserted());
            fdc.write(4, 0);
        }
        run_until(&mut fdc, |f| f.status_byte() == status::DONE);
        assert_eq!(fdc.read(1), result::OK);
        let sector = fdc.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(4, 0, 0)
            .unwrap();
        assert!(sector.iter().all(|&b| b == 0xE5));
    }
}
