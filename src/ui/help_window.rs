//! The Help and About windows (the Help menu, or F1 for Help).
//!
//! The words live in plain functions below the windows, so they are easy to
//! read, to keep up to date, and to test.

use eframe::egui::{self, RichText};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = "https://github.com/harrowm/atom-rs";

#[derive(Default)]
pub struct HelpWindows {
    pub help_open: bool,
    pub about_open: bool,
}

impl HelpWindows {
    pub fn show(&mut self, ctx: &egui::Context) {
        if self.help_open {
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("help"),
                egui::ViewportBuilder::default()
                    .with_title("atom-rs help")
                    .with_inner_size([620.0, 640.0]),
                |ui, _class| {
                    close = ui.input(|i| i.viewport().close_requested());
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, help_contents);
                    });
                },
            );
            self.help_open &= !close;
        }
        if self.about_open {
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("about"),
                egui::ViewportBuilder::default()
                    .with_title("About atom-rs")
                    .with_inner_size([520.0, 520.0]),
                |ui, _class| {
                    close = ui.input(|i| i.viewport().close_requested());
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, about_contents);
                    });
                },
            );
            self.about_open &= !close;
        }
    }
}

fn about_contents(ui: &mut egui::Ui) {
    ui.heading("atom-rs");
    ui.label(format!("Version {VERSION}"));
    ui.label("A cycle-accurate emulator of the Acorn Atom (1980), written in Rust as a learning project.");
    ui.hyperlink_to("github.com/harrowm/atom-rs", REPOSITORY);
    ui.separator();

    ui.label(RichText::new("Licence").strong());
    ui.label(licence_text());
    ui.separator();

    ui.label(RichText::new("Credits").strong());
    for &(name, what) in credits() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            ui.label(what);
        });
    }
    ui.separator();

    ui.label(RichText::new("The Atom's ROMs").strong());
    ui.label(rom_notice());
}

fn help_contents(ui: &mut egui::Ui) {
    ui.heading("Using atom-rs");
    ui.label("Open each heading below. F1 opens this window.");
    ui.add_space(6.0);

    for section in help_sections() {
        egui::CollapsingHeader::new(RichText::new(section.title).strong())
            .default_open(section.title == "Getting started")
            .show(ui, |ui| {
                for &paragraph in section.paragraphs {
                    ui.add(egui::Label::new(paragraph).wrap());
                }
            });
    }

    egui::CollapsingHeader::new(RichText::new("Keys").strong()).show(ui, |ui| {
        egui::Grid::new("help-keys").striped(true).show(ui, |ui| {
            for &(host, atom) in key_table() {
                ui.label(RichText::new(host).monospace());
                ui.label(atom);
                ui.end_row();
            }
        });
    });
}

// ---- The words ---------------------------------------------------------------------

struct Section {
    title: &'static str,
    paragraphs: &'static [&'static str],
}

fn help_sections() -> &'static [Section] {
    &[
        Section {
            title: "Getting started",
            paragraphs: &[
                "The Atom starts at its BASIC prompt (the > sign). Try: PRINT 2+2, or type a program such as 10 PRINT \"HELLO\" and then RUN.",
                "Commands starting with * go to the operating system: *CAT lists a disk or card.",
                "The emulator needs the Atom's ROMs in the roms folder (they come with it). If it says it cannot find them, use Hardware > ROMs > Choose ROM folder.",
                "From a terminal, atom-rs --archive downloads the Atom Software Archive (about 1,100 programs), keeps it up to date, and opens its menu.",
            ],
        },
        Section {
            title: "Loading software",
            paragraphs: &[
                "Tape: Tape > Insert tape (.uef or .csw), then type LOAD \"\" (or *RUN \"\"). The tape plays by itself, and Fast tape skips the waiting. Tape > Catalogue lists what is on the tape so you know what to LOAD.",
                "Floppy disk: Disk > Drive 0 > Insert disk image (.ssd or .dsd), type *DOS to start the disk system, then *. to see the files and *RUN NAME to start one. (Floppy machines only.)",
                "SD card: Hardware > Machine > SD card. The first time, the Software Archive is downloaded for you (about 16 MB; the status bar shows how it is going) and used as the card; to use a different folder, choose Hardware > Choose another card folder. *CAT lists the card, *LOAD and *SAVE work on its files, and *MENU starts the archive's menu.",
                "A program file: File > Load .atm file into memory, then File > Run loaded program.",
            ],
        },
        Section {
            title: "The four machines (Hardware > Machine)",
            paragraphs: &[
                "Floppy disk Atom: the standard disk Atom, and the default. You can choose how much RAM it has.",
                "Floppy disk Atom with GDOS/ADOS: the other disk system, with a WD1770 controller. Type *DOS to start it.",
                "SD card (AtoMMC): a folder on your computer is the SD card. It also has the RAM/ROM board, so utility ROMs from the archive load, and SDDOS can use disk images from the card.",
                "BBC BASIC: BBC BASIC 2 on the Atom, through the RAM/ROM board's BBC mode.",
                "Changing the machine restarts the Atom. The settings under the machine choice change to the ones that apply to it.",
            ],
        },
        Section {
            title: "What each menu does",
            paragraphs: &[
                "File: save and load the whole machine (save states; F6 and F7 are quick save and load), save a screenshot, load a program file, quit.",
                "Run: reset (F12, the BREAK key), pause, and the speed slider (up to 100 times real speed).",
                "Tape and Disk: tapes and floppy disk images. Disk is greyed out on machines with no floppy drives.",
                "Sound: mute and volume. Sound plays at normal speed or slower.",
                "Hardware: the kind of machine, its RAM, the video RAM, the colour board, the screen border, video snow, the SID sound card and its model, and the ROMs.",
                "Your choices (machine, RAM, sound, display and so on) are remembered next time. They are kept in ~/.config/atom-rs/settings.json; delete that file to start afresh.",
                "Input: the joystick on the keyboard (arrow keys and Space), and the keyboard mapping window.",
                "View: how the picture is scaled, and full screen. Debug: windows for the CPU, a disassembler, memory, breakpoints and the chips, and a command box where you type commands such as break 2900, s 10, set a 41 or help.",
            ],
        },
        Section {
            title: "If something is not working",
            paragraphs: &[
                "No sound: Sound > Mute may be on, or the speed is above 1x, where sound is switched off.",
                "Typing goes wrong straight after a restart: wait for the Atom's prompt before typing.",
                "A disk or tape will not load: check you are on a machine that has them (Disk is greyed out otherwise), and that you typed *DOS first for a disk.",
                "The screen is blank or odd in BBC BASIC: it is a graphics screen, so it looks different from the Atom's own text screen.",
                "A game cannot see its keys: try Input > Keyboard mapping, or switch the joystick on or off.",
            ],
        },
    ]
}

fn key_table() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "letters, digits, punctuation",
            "the Atom key in the same place (US layout; change it in Input > Keyboard mapping)",
        ),
        ("Shift, Ctrl", "Shift, Ctrl"),
        ("Alt (Option)", "REPT"),
        ("Enter, Backspace", "RETURN, DELETE"),
        (
            "Arrow keys",
            "the cursor keys (Down and Left are Shift with Up and Right, as on the Atom), or the joystick if Input > Joystick is on",
        ),
        ("Space (joystick on)", "fire, and still a space"),
        ("End, Tab", "COPY and ^"),
        ("Caps Lock, `", "LOCK"),
        ("F12", "BREAK (reset)"),
        ("F6, F7", "quick save, quick load of the whole machine"),
        ("F10", "save a screenshot (a PNG in the current folder)"),
        ("F11", "full screen on and off"),
        ("F1", "this help"),
    ]
}

fn licence_text() -> &'static str {
    "GNU General Public License, version 2 only. This follows from Atomulator, whose character font \
     is used here and whose memory map and behaviour this emulator was worked out from."
}

fn credits() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "Atomulator",
            "by Tom Walker and others, the reference for the memory map, the character font, the video, the AtoMMC and the add-on boards.",
        ),
        (
            "Tom Harte and others",
            "the SingleStepTests, which check the 6502 down to every cycle.",
        ),
        (
            "Klaus Dormann, Bruce Clark",
            "the 6502 functional, interrupt and decimal-mode tests.",
        ),
        ("sidera", "the SID sound chip emulator (MIT licence)."),
        (
            "Kees van Oss",
            "SDDOS, and many of the Atom conversions in the Software Archive.",
        ),
        (
            "The Atom Software Archive",
            "maintained by Hoglet, gathering software from stardot.org.uk.",
        ),
    ]
}

fn rom_notice() -> &'static str {
    "The ROM images in the roms folder are not part of this project's licence and are not its \
     authors' work: they are the Atom's system ROMs (copyright Acorn Computers), the AtoMMC, SDDOS \
     and BBC BASIC ROMs and the GDOS/ADOS ROM, copied unchanged from Atomulator, which has \
     distributed them so that the emulator can run."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_table_lists_the_special_keys() {
        let table: String = key_table()
            .iter()
            .map(|(host, _)| *host)
            .collect::<Vec<_>>()
            .join(" ");
        for key in ["F12", "F6, F7", "F1", "Alt", "Arrow keys"] {
            assert!(table.contains(key), "the table does not mention {key}");
        }
    }

    #[test]
    fn the_help_mentions_every_menu() {
        let text: String = help_sections()
            .iter()
            .flat_map(|section| section.paragraphs.iter().copied())
            .collect::<Vec<_>>()
            .join("\n");
        for menu in [
            "File:",
            "Run:",
            "Tape and Disk:",
            "Sound:",
            "Hardware:",
            "Input:",
            "View:",
        ] {
            assert!(
                text.contains(menu),
                "the menu guide does not describe {menu}"
            );
        }
    }

    #[test]
    fn the_help_names_all_four_machines() {
        use atom_rs::machine::atom::Machine;
        let text: String = help_sections()
            .iter()
            .filter(|section| section.title.contains("machines"))
            .flat_map(|section| section.paragraphs.iter().copied())
            .collect();
        for machine in Machine::ALL {
            assert!(
                text.contains(machine.label()),
                "the help does not describe {:?}",
                machine
            );
        }
    }

    #[test]
    fn the_about_text_says_what_the_licence_is() {
        assert!(licence_text().contains("version 2 only"));
        assert!(!VERSION.is_empty());
        assert!(rom_notice().contains("Acorn"));
    }

    /// Lay a window's contents out for a few frames with no window at all, to
    /// make sure drawing them does not panic.
    fn draws_without_panicking(contents: fn(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, contents);
            });
            // Nothing will draw this, so say so (egui insists on it).
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_help_window_draws() {
        draws_without_panicking(help_contents);
    }

    #[test]
    fn the_about_window_draws() {
        draws_without_panicking(about_contents);
    }
}
