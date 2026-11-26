//! The Western Digital WD1770 floppy disk controller, as fitted with GDOS.
//!
//! GDOS (and ADOS, its sibling) is a disk filing system for the Atom that uses
//! this chip instead of the Intel 8271 of the standard disk pack. It sits at
//! `BC10-BC13`, with a small control register of its own at `BC14`:
//!
//! | Address | Write | Read |
//! |---------|-------|------|
//! | `BC10`  | a command | the status |
//! | `BC11`  | the track register | the same |
//! | `BC12`  | the sector register | the same |
//! | `BC13`  | the next data byte | the next data byte |
//! | `BC14`  | control (below) | control, with the data request in bit 7 |
//!
//! The control register: bit 0 selects the side; bit 1 drive 0 (clear: drive
//! 1); bit 3 clear means double density; bit 4 **clear** resets the chip; bit
//! 5 chooses 40 or 80 tracks. Reading bit 6 gives the index pulse (it
//! alternates here, as in Atomulator, since nothing depends on its timing).
//!
//! The chip raises its interrupt request when a command finishes, which is
//! wired to the Atom's NMI. Bytes of data move one at a time, each announced
//! by the data request (status bit 1, and control bit 7), which the program
//! must answer before the next byte arrives.
//!
//! This works with whole sectors of the disk images the 8271 uses, with
//! simple timing: a head move or a sector search takes a few thousand cycles
//! and each byte 32 cycles (double density) or 64 (single). It does not model
//! the disk's rotation, CRC errors, or the precise layout of a formatted
//! track (formatting just clears the track).
//!
//! Adapted from Atomulator's `1770.c` (itself from Elkulator).

use super::disk::DiskImage;

const STATUS_REGISTER: u16 = 0xBC10;
const TRACK_REGISTER: u16 = 0xBC11;
const SECTOR_REGISTER: u16 = 0xBC12;
const DATA_REGISTER: u16 = 0xBC13;
const CONTROL_REGISTER: u16 = 0xBC14;
pub const FIRST_ADDRESS: u16 = STATUS_REGISTER;
pub const LAST_ADDRESS: u16 = CONTROL_REGISTER;

/// Status register bits.
mod status {
    pub const BUSY: u8 = 0x01;
    pub const DATA_REQUEST: u8 = 0x02;
    pub const TRACK_ZERO: u8 = 0x04;
    pub const RECORD_NOT_FOUND: u8 = 0x10;
    pub const WRITE_PROTECT: u8 = 0x40;
    pub const MOTOR_ON: u8 = 0x80;
}

/// Control register bits.
mod control {
    pub const SIDE: u8 = 0x01;
    pub const DRIVE_0: u8 = 0x02;
    /// Set: single density. Clear: double density (the control bit is "double
    /// density enable", active low).
    pub const SINGLE_DENSITY: u8 = 0x08;
    pub const NOT_RESET: u8 = 0x10;
    pub const INDEX: u8 = 0x40;
    pub const DATA_REQUEST: u8 = 0x80;
}

const SECTOR_SIZE: usize = 256;
const TRACKS: u8 = 80;
/// A formatted track holds about this many bytes (double density).
const TRACK_BYTES_DOUBLE_DENSITY: usize = 6250;
const TRACK_BYTES_SINGLE_DENSITY: usize = TRACK_BYTES_DOUBLE_DENSITY / 2;
/// Cycles for each byte to arrive from or go to the disk.
const BYTE_TIME_DOUBLE_DENSITY: u32 = 32;
const BYTE_TIME_SINGLE_DENSITY: u32 = 64;
/// Cycles for the head to step one track, and to settle.
const STEP_TIME: u32 = 3000;
const SETTLE_TIME: u32 = 1000;
/// From the last byte to the command finishing.
const FINISH_TIME: u32 = 200;

/// What the chip does once a delay is over.
#[derive(Clone, Copy, Debug)]
enum Then {
    /// A head move ends: the track register follows if `update_track`.
    HeadStopped {
        track_register: Option<u8>,
    },
    StartReading,
    StartWriting,
    StartReadingAddress,
    StartFormatting,
    Finish(u8),
    /// A multi-sector command moves on to the next sector.
    NextSector,
}

#[derive(Debug)]
enum Job {
    Idle,
    Delay {
        cycles: u32,
        then: Then,
    },
    /// Bytes going to the program (reading a sector or an address).
    Sending {
        bytes: Vec<u8>,
        next: usize,
        wait: u32,
        multiple: bool,
    },
    /// Bytes coming from the program (writing a sector, or a track).
    Receiving {
        bytes: Vec<u8>,
        expected: usize,
        wait: u32,
        writing_track: bool,
        multiple: bool,
    },
}

pub struct Fdc1770 {
    track: u8,
    sector: u8,
    data: u8,
    command: u8,
    /// What the status shows when no command is running.
    result: u8,
    control: u8,
    /// The physical head position, which the track register should match.
    head: u8,
    step_in: bool,
    index_pulse: bool,
    data_request: bool,
    interrupt_request: bool,
    job: Job,
}

impl Default for Fdc1770 {
    fn default() -> Self {
        Fdc1770 {
            track: 0,
            sector: 0,
            data: 0,
            command: 0,
            result: 0,
            control: control::NOT_RESET | control::DRIVE_0,
            head: 0,
            step_in: true,
            index_pulse: false,
            data_request: false,
            interrupt_request: false,
            job: Job::Idle,
        }
    }
}

impl Fdc1770 {
    pub fn new() -> Self {
        Self::default()
    }

    /// The interrupt request line, which is the Atom's NMI.
    pub fn nmi_asserted(&self) -> bool {
        self.interrupt_request
    }

    pub fn reset(&mut self) {
        *self = Fdc1770 {
            control: self.control,
            head: self.head,
            ..Fdc1770::default()
        };
    }

    // ---- What the program sees ---------------------------------------------------

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            STATUS_REGISTER => {
                self.interrupt_request = false;
                self.status()
            }
            TRACK_REGISTER => self.track,
            SECTOR_REGISTER => self.sector,
            DATA_REGISTER => self.read_data(),
            CONTROL_REGISTER => {
                self.index_pulse = !self.index_pulse;
                let mut value = self.control & !(control::INDEX | control::DATA_REQUEST);
                if self.index_pulse {
                    value |= control::INDEX;
                }
                if self.data_request {
                    value |= control::DATA_REQUEST;
                }
                value
            }
            _ => 0xFE,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8, drives: &mut [Option<DiskImage>; 2]) {
        match addr {
            STATUS_REGISTER => self.command(value),
            TRACK_REGISTER => self.track = value,
            SECTOR_REGISTER => self.sector = value,
            DATA_REGISTER => self.write_data(value, drives),
            CONTROL_REGISTER => {
                self.control = value;
                if value & control::NOT_RESET == 0 {
                    self.reset();
                }
            }
            _ => {}
        }
    }

    fn status(&self) -> u8 {
        match &self.job {
            Job::Idle => self.result,
            _ => {
                let mut status = status::MOTOR_ON | status::BUSY;
                if self.data_request {
                    status |= status::DATA_REQUEST;
                }
                status
            }
        }
    }

    fn side(&self) -> u8 {
        self.control & control::SIDE
    }

    fn drive(&self) -> usize {
        if self.control & control::DRIVE_0 != 0 {
            0
        } else {
            1
        }
    }

    fn double_density(&self) -> bool {
        self.control & control::SINGLE_DENSITY == 0
    }

    fn byte_time(&self) -> u32 {
        if self.double_density() {
            BYTE_TIME_DOUBLE_DENSITY
        } else {
            BYTE_TIME_SINGLE_DENSITY
        }
    }

    /// The status of a finished head move: motor on, and track zero if there.
    fn head_status(&self) -> u8 {
        status::MOTOR_ON
            | if self.head == 0 {
                status::TRACK_ZERO
            } else {
                0
            }
    }

    // ---- Commands ------------------------------------------------------------------

    fn command(&mut self, value: u8) {
        let force_interrupt = value >> 4 == 0xD;
        if !matches!(self.job, Job::Idle) && !force_interrupt {
            return; // busy: the chip ignores it
        }
        self.command = value;
        self.interrupt_request = false;
        self.data_request = false;
        match value >> 4 {
            0x0 => self.start_head_move(0, true),
            0x1 => {
                let target = self.data;
                self.start_head_move(target, true);
            }
            0x2 | 0x3 => {
                let target = self.stepped(self.step_in);
                self.start_head_move(target, value & 0x10 != 0);
            }
            0x4 | 0x5 => {
                self.step_in = true;
                let target = self.stepped(true);
                self.start_head_move(target, value & 0x10 != 0);
            }
            0x6 | 0x7 => {
                self.step_in = false;
                let target = self.stepped(false);
                self.start_head_move(target, value & 0x10 != 0);
            }
            0x8 | 0x9 => self.delay(SETTLE_TIME, Then::StartReading),
            0xA | 0xB => self.delay(SETTLE_TIME, Then::StartWriting),
            0xC => self.delay(SETTLE_TIME, Then::StartReadingAddress),
            0xD => {
                // Force interrupt: stop whatever is happening.
                self.job = Job::Idle;
                self.data_request = false;
                self.result = self.head_status();
                self.interrupt_request = true;
            }
            0xF => self.delay(SETTLE_TIME, Then::StartFormatting),
            _ => {
                // Read track (not emulated): report the sector as not found.
                self.finish(status::RECORD_NOT_FOUND);
            }
        }
    }

    /// Where the head goes for a step command.
    fn stepped(&self, inwards: bool) -> u8 {
        if inwards {
            self.head.saturating_add(1).min(TRACKS - 1)
        } else {
            self.head.saturating_sub(1)
        }
    }

    fn start_head_move(&mut self, target: u8, update_track_register: bool) {
        let target = target.min(TRACKS - 1);
        let moved = u32::from(self.head.abs_diff(target));
        self.head = target;
        self.job = Job::Delay {
            cycles: SETTLE_TIME + moved * STEP_TIME,
            then: Then::HeadStopped {
                track_register: update_track_register.then_some(target),
            },
        };
    }

    fn delay(&mut self, cycles: u32, then: Then) {
        self.job = Job::Delay { cycles, then };
    }

    /// The chip's job is done: show the result and raise the interrupt.
    fn finish(&mut self, bits: u8) {
        self.job = Job::Idle;
        self.data_request = false;
        self.result = status::MOTOR_ON | bits;
        self.interrupt_request = true;
    }

    // ---- Data ------------------------------------------------------------------------

    fn read_data(&mut self) -> u8 {
        let value = self.data;
        self.data_request = false;
        if let Job::Sending {
            bytes,
            next,
            multiple,
            ..
        } = &self.job
            && *next >= bytes.len()
        {
            // The last byte has been taken.
            let then = if *multiple {
                Then::NextSector
            } else {
                Then::Finish(0)
            };
            self.job = Job::Delay {
                cycles: FINISH_TIME,
                then,
            };
        }
        value
    }

    fn write_data(&mut self, value: u8, drives: &mut [Option<DiskImage>; 2]) {
        self.data = value;
        if !self.data_request {
            return;
        }
        self.data_request = false;
        let byte_time = self.byte_time();
        let Job::Receiving {
            bytes,
            expected,
            wait,
            ..
        } = &mut self.job
        else {
            return;
        };
        bytes.push(value);
        *wait = byte_time;
        if bytes.len() >= *expected {
            self.finish_receiving(drives);
        }
    }

    /// All the bytes of a write have come in: put them on the disk.
    fn finish_receiving(&mut self, drives: &mut [Option<DiskImage>; 2]) {
        let Job::Receiving {
            bytes,
            writing_track,
            multiple,
            ..
        } = std::mem::replace(&mut self.job, Job::Idle)
        else {
            return;
        };
        let (drive, side, head) = (self.drive(), self.side(), self.head);
        let Some(disk) = drives[drive].as_mut() else {
            return self.finish(status::RECORD_NOT_FOUND);
        };
        if writing_track {
            disk.format_track(head, side, 0xE5);
            return self.finish(0);
        }
        if !disk.write_sector(head, side, self.sector, &bytes) {
            return self.finish(status::RECORD_NOT_FOUND);
        }
        let then = if multiple {
            Then::NextSector
        } else {
            Then::Finish(0)
        };
        self.job = Job::Delay {
            cycles: FINISH_TIME,
            then,
        };
    }

    // ---- Time passing ------------------------------------------------------------------

    /// One CPU cycle.
    pub fn tick(&mut self, drives: &mut [Option<DiskImage>; 2]) {
        let byte_time = self.byte_time();
        match &mut self.job {
            Job::Idle => {}
            Job::Delay { cycles, then } => {
                *cycles = cycles.saturating_sub(1);
                if *cycles == 0 {
                    let then = *then;
                    self.job = Job::Idle;
                    self.after_delay(then, drives);
                }
            }
            Job::Sending {
                bytes, next, wait, ..
            } => {
                if self.data_request {
                    return; // the program has not taken the last byte yet
                }
                *wait = wait.saturating_sub(1);
                if *wait == 0 && *next < bytes.len() {
                    self.data = bytes[*next];
                    *next += 1;
                    self.data_request = true;
                    *wait = byte_time;
                }
            }
            Job::Receiving {
                wait,
                expected,
                bytes,
                ..
            } => {
                if self.data_request || bytes.len() >= *expected {
                    return;
                }
                *wait = wait.saturating_sub(1);
                if *wait == 0 {
                    self.data_request = true;
                }
            }
        }
    }

    fn after_delay(&mut self, then: Then, drives: &mut [Option<DiskImage>; 2]) {
        match then {
            Then::HeadStopped { track_register } => {
                if let Some(target) = track_register {
                    self.track = target;
                }
                self.finish(self.head_status() & !status::MOTOR_ON);
            }
            Then::Finish(bits) => self.finish(bits),
            Then::StartReading => self.start_reading(drives),
            Then::StartWriting => self.start_writing(drives),
            Then::StartReadingAddress => {
                // The first ID the head meets: track, side, sector, size code
                // (1 = 256 bytes) and two CRC bytes. The chip also copies the
                // track number into the sector register.
                let bytes = vec![self.head, self.side(), 0, 1, 0, 0];
                self.sector = self.head;
                self.job = Job::Sending {
                    bytes,
                    next: 0,
                    wait: 1,
                    multiple: false,
                };
            }
            Then::StartFormatting => {
                if drives[self.drive()]
                    .as_ref()
                    .is_some_and(|d| d.write_protected)
                {
                    return self.finish(status::WRITE_PROTECT);
                }
                let expected = if self.double_density() {
                    TRACK_BYTES_DOUBLE_DENSITY
                } else {
                    TRACK_BYTES_SINGLE_DENSITY
                };
                self.job = Job::Receiving {
                    bytes: Vec::new(),
                    expected,
                    wait: 1,
                    writing_track: true,
                    multiple: false,
                };
                self.data_request = true;
            }
            Then::NextSector => {
                self.sector = self.sector.wrapping_add(1);
                if self.command >> 4 == 0x9 {
                    self.start_reading(drives);
                } else {
                    self.start_writing(drives);
                }
            }
        }
    }

    fn start_reading(&mut self, drives: &mut [Option<DiskImage>; 2]) {
        // The sector must be on the track the head is over, and the track
        // register must say that is where it is.
        let found = (self.track == self.head)
            .then(|| drives[self.drive()].as_ref())
            .flatten()
            .and_then(|disk| disk.read_sector(self.head, self.side(), self.sector));
        match found {
            Some(bytes) => {
                self.job = Job::Sending {
                    bytes: bytes.to_vec(),
                    next: 0,
                    wait: 1,
                    multiple: self.command & 0x10 != 0,
                };
            }
            None => self.finish(status::RECORD_NOT_FOUND),
        }
    }

    fn start_writing(&mut self, drives: &mut [Option<DiskImage>; 2]) {
        let Some(disk) = drives[self.drive()].as_ref() else {
            return self.finish(status::RECORD_NOT_FOUND);
        };
        if disk.write_protected {
            return self.finish(status::WRITE_PROTECT);
        }
        if self.track != self.head
            || disk
                .read_sector(self.head, self.side(), self.sector)
                .is_none()
        {
            return self.finish(status::RECORD_NOT_FOUND);
        }
        self.job = Job::Receiving {
            bytes: Vec::with_capacity(SECTOR_SIZE),
            expected: SECTOR_SIZE,
            wait: 1,
            writing_track: false,
            multiple: self.command & 0x10 != 0,
        };
        self.data_request = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disk in drive 0 in which every byte of sector `s` on track `t` is
    /// `t * 16 + s`.
    fn patterned_disk() -> DiskImage {
        let mut disk = DiskImage::blank_80_track(1);
        for track in 0..80u8 {
            for sector in 0..10u8 {
                disk.write_sector(
                    track,
                    0,
                    sector,
                    &[track.wrapping_mul(16).wrapping_add(sector); 256],
                );
            }
        }
        disk
    }

    struct Rig {
        chip: Fdc1770,
        drives: [Option<DiskImage>; 2],
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                chip: Fdc1770::new(),
                drives: [Some(patterned_disk()), None],
            }
        }

        fn write(&mut self, addr: u16, value: u8) {
            self.chip.write(addr, value, &mut self.drives);
        }

        fn run(&mut self, cycles: u32) {
            for _ in 0..cycles {
                self.chip.tick(&mut self.drives);
            }
        }

        /// Run until the command is over (the interrupt request is raised).
        fn run_to_the_end(&mut self) {
            for _ in 0..2_000_000 {
                if self.chip.nmi_asserted() {
                    return;
                }
                self.chip.tick(&mut self.drives);
            }
            panic!("the command never finished");
        }

        /// Run until the data request, then take the byte.
        fn next_byte(&mut self) -> u8 {
            for _ in 0..10_000 {
                if self.chip.read(CONTROL_REGISTER) & control::DATA_REQUEST != 0 {
                    return self.chip.read(DATA_REGISTER);
                }
                self.chip.tick(&mut self.drives);
            }
            panic!("no data request");
        }

        /// Do a command and wait for it to end, returning the status.
        fn command(&mut self, command: u8) -> u8 {
            self.write(STATUS_REGISTER, command);
            self.run_to_the_end();
            self.chip.read(STATUS_REGISTER)
        }

        fn go_to_track(&mut self, track: u8) {
            self.write(DATA_REGISTER, track);
            self.command(0x10);
        }
    }

    #[test]
    fn restore_puts_the_head_at_track_zero() {
        let mut rig = Rig::new();
        rig.go_to_track(7);
        assert_eq!(rig.chip.head, 7);
        let status = rig.command(0x00);
        assert_eq!(rig.chip.head, 0);
        assert_eq!(rig.chip.track, 0);
        assert_ne!(status & status::TRACK_ZERO, 0, "TR00");
        assert_eq!(status & status::BUSY, 0);
    }

    #[test]
    fn seek_moves_the_head_and_the_track_register() {
        let mut rig = Rig::new();
        let status = rig.command_seek(12);
        assert_eq!((rig.chip.head, rig.chip.track), (12, 12));
        assert_eq!(status & status::TRACK_ZERO, 0, "not at track zero");
    }

    impl Rig {
        fn command_seek(&mut self, track: u8) -> u8 {
            self.write(DATA_REGISTER, track);
            self.command(0x10)
        }
    }

    #[test]
    fn a_finished_command_raises_the_interrupt_until_the_status_is_read() {
        let mut rig = Rig::new();
        rig.write(STATUS_REGISTER, 0x00);
        assert!(!rig.chip.nmi_asserted());
        rig.run_to_the_end();
        assert!(rig.chip.nmi_asserted());
        rig.chip.read(STATUS_REGISTER);
        assert!(!rig.chip.nmi_asserted(), "reading the status clears it");
    }

    #[test]
    fn steps_in_and_out_one_track_at_a_time() {
        let mut rig = Rig::new();
        rig.command(0x50); // step in, updating the track register
        rig.command(0x50);
        assert_eq!((rig.chip.head, rig.chip.track), (2, 2));
        rig.command(0x70); // step out
        assert_eq!((rig.chip.head, rig.chip.track), (1, 1));
    }

    #[test]
    fn reads_a_sector_byte_by_byte_with_a_data_request_for_each() {
        let mut rig = Rig::new();
        rig.go_to_track(3);
        rig.write(SECTOR_REGISTER, 4);
        rig.write(STATUS_REGISTER, 0x80);
        assert_ne!(rig.chip.read(STATUS_REGISTER) & status::BUSY, 0);
        let bytes: Vec<u8> = (0..256).map(|_| rig.next_byte()).collect();
        assert!(bytes.iter().all(|&b| b == 3 * 16 + 4), "track 3, sector 4");
        rig.run_to_the_end();
        let status = rig.chip.read(STATUS_REGISTER);
        assert_eq!(status & !status::MOTOR_ON, 0, "no errors, not busy");
    }

    #[test]
    fn a_sector_that_is_not_there_is_record_not_found() {
        let mut rig = Rig::new();
        rig.write(SECTOR_REGISTER, 40);
        let status = rig.command(0x80);
        assert_ne!(status & status::RECORD_NOT_FOUND, 0);
    }

    #[test]
    fn reading_with_the_wrong_track_register_is_record_not_found() {
        let mut rig = Rig::new();
        rig.go_to_track(5);
        rig.write(TRACK_REGISTER, 9); // the chip thinks it is somewhere else
        rig.write(SECTOR_REGISTER, 0);
        let status = rig.command(0x80);
        assert_ne!(status & status::RECORD_NOT_FOUND, 0);
    }

    #[test]
    fn reading_with_no_disk_in_the_drive_is_record_not_found() {
        let mut rig = Rig::new();
        rig.write(CONTROL_REGISTER, control::NOT_RESET); // drive 1: empty
        rig.write(SECTOR_REGISTER, 0);
        let status = rig.command(0x80);
        assert_ne!(status & status::RECORD_NOT_FOUND, 0);
    }

    #[test]
    fn a_multiple_sector_read_carries_on_until_the_track_runs_out() {
        let mut rig = Rig::new();
        rig.write(SECTOR_REGISTER, 8);
        rig.write(STATUS_REGISTER, 0x90);
        let first: Vec<u8> = (0..256).map(|_| rig.next_byte()).collect();
        let second: Vec<u8> = (0..256).map(|_| rig.next_byte()).collect();
        assert!(first.iter().all(|&b| b == 8) && second.iter().all(|&b| b == 9));
        rig.run_to_the_end();
        let status = rig.chip.read(STATUS_REGISTER);
        assert_ne!(status & status::RECORD_NOT_FOUND, 0, "no sector 10");
        assert_eq!(rig.chip.sector, 10);
    }

    #[test]
    fn writes_a_sector_to_the_disk() {
        let mut rig = Rig::new();
        rig.go_to_track(2);
        rig.write(SECTOR_REGISTER, 6);
        rig.write(STATUS_REGISTER, 0xA0);
        for i in 0..256u32 {
            // The data request comes first, then the program answers it.
            for _ in 0..10_000 {
                if rig.chip.read(CONTROL_REGISTER) & control::DATA_REQUEST != 0 {
                    break;
                }
                rig.chip.tick(&mut rig.drives);
            }
            rig.write(DATA_REGISTER, i as u8);
        }
        rig.run_to_the_end();
        assert_eq!(rig.chip.read(STATUS_REGISTER) & !status::MOTOR_ON, 0);
        let written = rig.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(2, 0, 6)
            .unwrap();
        assert_eq!(written, (0..=255).collect::<Vec<u8>>());
        let neighbour = rig.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(2, 0, 7)
            .unwrap();
        assert!(neighbour.iter().all(|&b| b == 2 * 16 + 7), "untouched");
    }

    #[test]
    fn a_write_protected_disk_refuses_writes() {
        let mut rig = Rig::new();
        rig.drives[0].as_mut().unwrap().write_protected = true;
        rig.write(SECTOR_REGISTER, 0);
        let status = rig.command(0xA0);
        assert_ne!(status & status::WRITE_PROTECT, 0);
    }

    #[test]
    fn read_address_gives_the_track_and_side_and_copies_the_track_to_the_sector_register() {
        let mut rig = Rig::new();
        rig.go_to_track(9);
        rig.write(STATUS_REGISTER, 0xC0);
        let id: Vec<u8> = (0..6).map(|_| rig.next_byte()).collect();
        assert_eq!(id[0], 9, "track");
        assert_eq!(id[1], 0, "side");
        assert_eq!(id[3], 1, "256-byte sectors");
        rig.run_to_the_end();
        assert_eq!(rig.chip.sector, 9);
    }

    #[test]
    fn force_interrupt_stops_a_command_and_raises_the_interrupt() {
        let mut rig = Rig::new();
        rig.write(SECTOR_REGISTER, 0);
        rig.write(STATUS_REGISTER, 0x80);
        rig.run(2000);
        rig.write(STATUS_REGISTER, 0xD0);
        assert!(rig.chip.nmi_asserted());
        assert_eq!(rig.chip.read(STATUS_REGISTER) & status::BUSY, 0);
        assert_eq!(rig.chip.read(CONTROL_REGISTER) & control::DATA_REQUEST, 0);
    }

    #[test]
    fn a_command_given_while_busy_is_ignored() {
        let mut rig = Rig::new();
        rig.write(DATA_REGISTER, 20);
        rig.write(STATUS_REGISTER, 0x10); // seek to 20
        rig.write(STATUS_REGISTER, 0x00); // restore: ignored, still seeking
        rig.run_to_the_end();
        assert_eq!(rig.chip.head, 20);
    }

    #[test]
    fn the_control_register_picks_drive_and_side_and_reset_clears_the_chip() {
        let mut rig = Rig::new();
        rig.write(
            CONTROL_REGISTER,
            control::NOT_RESET | control::DRIVE_0 | control::SIDE,
        );
        assert_eq!((rig.chip.drive(), rig.chip.side()), (0, 1));
        rig.write(CONTROL_REGISTER, control::NOT_RESET);
        assert_eq!(rig.chip.drive(), 1);
        rig.write(TRACK_REGISTER, 33);
        rig.write(CONTROL_REGISTER, 0); // bit 4 clear: reset
        assert_eq!(rig.chip.read(TRACK_REGISTER), 0);
    }

    #[test]
    fn the_index_bit_alternates_on_reading_the_control_register() {
        let mut rig = Rig::new();
        let a = rig.chip.read(CONTROL_REGISTER) & control::INDEX;
        let b = rig.chip.read(CONTROL_REGISTER) & control::INDEX;
        assert_ne!(a, b);
    }

    #[test]
    fn writing_a_track_formats_it() {
        let mut rig = Rig::new();
        rig.go_to_track(4);
        rig.write(STATUS_REGISTER, 0xF0);
        for _ in 0..TRACK_BYTES_DOUBLE_DENSITY {
            for _ in 0..10_000 {
                if rig.chip.read(CONTROL_REGISTER) & control::DATA_REQUEST != 0 {
                    break;
                }
                rig.chip.tick(&mut rig.drives);
            }
            rig.write(DATA_REGISTER, 0xFF);
        }
        rig.run_to_the_end();
        let sector = rig.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(4, 0, 3)
            .unwrap();
        assert!(sector.iter().all(|&b| b == 0xE5), "formatted");
        let elsewhere = rig.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(5, 0, 3)
            .unwrap();
        assert!(
            elsewhere.iter().all(|&b| b == 5 * 16 + 3),
            "other tracks are untouched"
        );
    }

    #[test]
    fn single_density_is_slower_than_double() {
        let mut chip = Fdc1770::new();
        chip.control = control::NOT_RESET; // DDEN clear: double
        assert_eq!(chip.byte_time(), BYTE_TIME_DOUBLE_DENSITY);
        chip.control = control::NOT_RESET | control::SINGLE_DENSITY;
        assert_eq!(chip.byte_time(), BYTE_TIME_SINGLE_DENSITY);
    }
}
