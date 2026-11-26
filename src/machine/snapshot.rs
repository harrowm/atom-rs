//! Save states: freeze the whole machine to a file and bring it back later.
//!
//! A snapshot holds everything that makes the machine *this* machine right
//! now: the CPU, all of RAM, every chip, the disks in the drives and the tape.
//! It deliberately leaves out the ROMs (they come from files and never
//! change), the debugger's breakpoints, and the picture (which is redrawn).
//!
//! The file is JSON, compressed with gzip. JSON is bulky for 64K of RAM but it
//! is easy to inspect, and compression takes most of the sting out.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use super::atom::Atom;
use super::fdc8271::Fdc8271;
use super::ppi8255::Ppi;
use super::tape::Tape;
use super::vdg6847::{VISIBLE_LINES, Vdg, VideoMode};
use super::via6522::Via;
use crate::cpu::Cpu;

/// Bump this when the snapshot contents change in an incompatible way.
const VERSION: u32 = 1;

const RAM_SIZE: usize = 0x10000;

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    cpu: Cpu,
    ram: Vec<u8>,
    ppi: Ppi,
    via: Via,
    vdg: Vdg,
    fdc: Fdc8271,
    tape: Tape,
    cycles: u64,
    external_irq: bool,
    nmi_edge: bool,
    fdc_nmi_was_asserted: bool,
    /// The RAM/ROM board's bank and flags registers, if one is fitted.
    #[serde(default)]
    ramrom_registers: Option<(u8, u8)>,
}

impl Atom {
    /// Capture the machine as bytes (a gzip-compressed snapshot).
    pub fn save_state(&self) -> Result<Vec<u8>, String> {
        let bus = &self.bus;
        let snapshot = Snapshot {
            version: VERSION,
            cpu: self.cpu.clone(),
            ram: bus.memory.ram.to_vec(),
            ppi: bus.ppi.clone(),
            via: bus.via.clone(),
            vdg: bus.vdg.clone(),
            fdc: bus.fdc.clone(),
            tape: bus.tape.clone(),
            cycles: bus.cycles,
            external_irq: bus.external_irq,
            nmi_edge: bus.nmi_edge,
            fdc_nmi_was_asserted: bus.fdc_nmi_was_asserted,
            ramrom_registers: bus.memory.board.as_ref().map(|board| board.registers()),
        };
        let json = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&json).map_err(|e| e.to_string())?;
        encoder.finish().map_err(|e| e.to_string())
    }

    /// Restore a snapshot made by [`Atom::save_state`]. If the data is bad the
    /// machine is left untouched.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_end(&mut json)
            .map_err(|e| format!("not a saved state: {e}"))?;
        let snapshot: Snapshot =
            serde_json::from_slice(&json).map_err(|e| format!("damaged saved state: {e}"))?;
        if snapshot.version != VERSION {
            return Err(format!(
                "saved state is version {}, this program reads version {VERSION}",
                snapshot.version
            ));
        }
        if snapshot.ram.len() != RAM_SIZE {
            return Err("saved state has the wrong amount of RAM".to_string());
        }

        let bus = &mut self.bus;
        self.cpu = snapshot.cpu;
        bus.memory.ram.copy_from_slice(&snapshot.ram);
        bus.ppi = snapshot.ppi;
        bus.via = snapshot.via;
        bus.vdg = snapshot.vdg;
        bus.fdc = snapshot.fdc;
        bus.tape = snapshot.tape;
        if let (Some(board), Some((bank, flags))) =
            (&mut bus.memory.board, snapshot.ramrom_registers)
        {
            board.set_registers(bank, flags);
        }
        // The SID chip's state is not saved: it starts silent.
        bus.sid.reset();
        bus.cycles = snapshot.cycles;
        bus.external_irq = snapshot.external_irq;
        bus.nmi_edge = snapshot.nmi_edge;
        bus.fdc_nmi_was_asserted = snapshot.fdc_nmi_was_asserted;
        self.redraw_screen();
        Ok(())
    }

    /// Redraw the whole picture from video RAM (after loading a state).
    fn redraw_screen(&mut self) {
        let bus = &mut self.bus;
        let mode = VideoMode::from_port_a_high_nibble(bus.ppi.video_mode_bits());
        for line in 0..VISIBLE_LINES as usize {
            bus.framebuffer
                .render_line(line, &bus.memory.ram[..], mode, bus.ppi.colour_set());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restored_machine_continues_exactly_as_the_original() {
        let mut atom = Atom::new();
        // A little program in RAM that counts forever: INC $10 ; JMP $0100
        atom.bus.memory.ram[0x0100..0x0105].copy_from_slice(&[0xE6, 0x10, 0x4C, 0x00, 0x01]);
        atom.cpu.regs.pc = 0x0100;
        atom.run_cycles(10_000);

        let state = atom.save_state().unwrap();
        atom.run_cycles(50_000);
        let counter_after = atom.bus.memory.ram[0x10];
        let cycles_after = atom.bus.cycles;

        // Different machine, restored: must reach the same place.
        let mut other = Atom::new();
        other.load_state(&state).unwrap();
        other.run_cycles(50_000);
        assert_eq!(other.bus.memory.ram[0x10], counter_after);
        assert_eq!(other.bus.cycles, cycles_after);
        assert_eq!(other.cpu.regs, atom.cpu.regs);
    }

    #[test]
    fn disks_and_tape_are_part_of_the_state() {
        use crate::machine::disk::DiskImage;
        let mut atom = Atom::new();
        let mut disk = DiskImage::blank_80_track(1);
        disk.write_sector(2, 0, 3, &[0x5A; 256]);
        atom.bus.fdc.drives[0] = Some(disk);

        let state = atom.save_state().unwrap();
        let mut other = Atom::new();
        other.load_state(&state).unwrap();
        let sector = other.bus.fdc.drives[0]
            .as_ref()
            .unwrap()
            .read_sector(2, 0, 3)
            .unwrap();
        assert!(sector.iter().all(|&b| b == 0x5A));
    }

    #[test]
    fn bad_data_is_rejected_and_leaves_the_machine_alone() {
        let mut atom = Atom::new();
        atom.cpu.regs.a = 0x42;
        assert!(atom.load_state(b"definitely not a state").is_err());
        assert_eq!(atom.cpu.regs.a, 0x42);
    }
}
