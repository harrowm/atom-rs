//! The state of the Atom's chips: video, 8255 and 6522.

use atom_rs::machine::Atom;
use eframe::egui;

pub fn show(ui: &mut egui::Ui, atom: &Atom) {
    let bus = &atom.bus;

    ui.heading("Video (6847)");
    ui.monospace(format!("Mode: {:?}", atom.video_mode()));
    ui.monospace(format!("Colour set: {}", bus.ppi.colour_set() as u8));
    ui.monospace(format!(
        "Frame {}  line {}  FS {}",
        bus.vdg.frames,
        bus.vdg.line(),
        if bus.vdg.field_sync_low() {
            "low (sync)"
        } else {
            "high"
        }
    ));

    ui.separator();
    ui.heading("8255 PPI (B000)");
    ui.monospace(format!(
        "Port A (row/mode): {:02X}   keyboard row {}",
        bus.ppi.port_a(),
        bus.ppi.port_a() & 0x0F
    ));
    ui.monospace(format!("Port B (keys):     {:02X}", bus.ppi.read(1)));
    ui.monospace(format!("Port C:            {:02X}", bus.ppi.read(2)));
    ui.monospace(format!("Speaker: {}", bus.ppi.speaker() as u8));

    ui.separator();
    ui.heading("6522 VIA (B800)");
    let via = bus.via.snapshot();
    ui.monospace(format!(
        "ORA {:02X} DDRA {:02X}   ORB {:02X} DDRB {:02X}",
        via.output_a, via.direction_a, via.output_b, via.direction_b
    ));
    ui.monospace(format!(
        "T1 {:04X} (latch {:04X})   T2 {:04X}",
        via.timer1, via.timer1_latch, via.timer2
    ));
    ui.monospace(format!(
        "ACR {:02X}  PCR {:02X}",
        via.aux_control, via.peripheral_control
    ));
    ui.monospace(format!(
        "IFR {:02X}  IER {:02X}  IRQ line {}",
        via.flags,
        via.enables,
        if bus.via.irq() || bus.external_irq {
            "asserted"
        } else {
            "clear"
        }
    ));

    ui.separator();
    ui.heading("8271 disk controller (0A00)");
    if !bus.memory.disk_interface_fitted {
        ui.label("Not fitted (no disk ROM loaded)");
        return;
    }
    let fdc = &bus.fdc;
    ui.monospace(format!(
        "Status {:02X}  {}  NMI {}",
        fdc.status_byte(),
        if fdc.is_busy() { "busy" } else { "idle" },
        if fdc.nmi_asserted() {
            "asserted"
        } else {
            "clear"
        }
    ));
    for drive in 0..2 {
        let disk = match &fdc.drives[drive] {
            Some(disk) => format!("{} tracks, {} side(s)", disk.tracks, disk.sides),
            None => "empty".to_string(),
        };
        ui.monospace(format!(
            "Drive {drive}: head at track {:2}  {disk}",
            fdc.current_track(drive)
        ));
    }
}
