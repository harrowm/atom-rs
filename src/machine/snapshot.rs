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
use super::memory::RamPreset;
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
    /// How much RAM is fitted.
    #[serde(default)]
    ram_preset: RamPreset,
    /// How much video RAM is fitted, in K.
    #[serde(default = "full_video_ram")]
    video_ram_kb: u8,
}

fn full_video_ram() -> u8 {
    super::memory::MAX_VIDEO_RAM_KB
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
            ram_preset: bus.memory.ram_preset(),
            video_ram_kb: bus.memory.video_ram_kb(),
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
        bus.memory.set_ram_preset(snapshot.ram_preset);
        bus.memory.set_video_ram_kb(snapshot.video_ram_kb);
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
        // The GDOS disk controller's state is not saved: it starts idle.
        bus.fdc1770.reset();
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

    // ---- Damaged and old saved states ----------------------------------------------------------------

    /// A saved state edited as JSON: gunzip, change, gzip again.
    fn edited_state(atom: &Atom, edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(&atom.save_state().unwrap()[..])
            .read_to_end(&mut json)
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        edit(&mut value);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder
            .write_all(&serde_json::to_vec(&value).unwrap())
            .unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn a_state_from_another_version_is_refused_and_the_machine_left_alone() {
        let mut atom = Atom::new();
        let state = edited_state(&atom, |v| v["version"] = 99.into());
        atom.cpu.regs.a = 0x42;
        let message = atom.load_state(&state).unwrap_err();
        assert!(message.contains("version 99"), "{message}");
        assert_eq!(atom.cpu.regs.a, 0x42);
    }

    #[test]
    fn a_state_with_the_wrong_amount_of_ram_is_refused_and_the_machine_left_alone() {
        let mut atom = Atom::new();
        let state = edited_state(&atom, |v| v["ram"] = serde_json::json!([1, 2, 3]));
        atom.cpu.regs.a = 0x42;
        assert!(
            atom.load_state(&state)
                .unwrap_err()
                .contains("wrong amount of RAM")
        );
        assert_eq!(atom.cpu.regs.a, 0x42);
    }

    #[test]
    fn a_state_that_is_gzip_but_not_json_is_called_damaged() {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(b"{ this is not json").unwrap();
        let junk = encoder.finish().unwrap();
        let mut atom = Atom::new();
        assert!(atom.load_state(&junk).unwrap_err().contains("damaged"));
        // And one with a missing field.
        let state = edited_state(&atom, |v| {
            v.as_object_mut().unwrap().remove("cpu");
        });
        assert!(atom.load_state(&state).unwrap_err().contains("damaged"));
    }

    #[test]
    fn a_cut_off_state_is_refused() {
        let atom = Atom::new();
        let state = atom.save_state().unwrap();
        let mut other = Atom::new();
        assert!(other.load_state(&state[..state.len() / 2]).is_err());
        assert!(other.load_state(&[]).is_err());
        assert!(other.load_state(&state[..10]).is_err());
    }

    #[test]
    fn a_state_saved_before_video_ram_and_ram_size_were_recorded_still_loads() {
        let mut atom = Atom::new();
        atom.bus.memory.set_video_ram_kb(3);
        let old = edited_state(&atom, |v| {
            let fields = v.as_object_mut().unwrap();
            fields.remove("video_ram_kb");
            fields.remove("ram_preset");
            fields.remove("ramrom_registers");
        });
        let mut other = Atom::new();
        other.bus.memory.set_video_ram_kb(2);
        other.load_state(&old).unwrap();
        assert_eq!(
            other.bus.memory.video_ram_kb(),
            crate::machine::memory::MAX_VIDEO_RAM_KB,
            "an old state means the full video RAM"
        );
    }
}
