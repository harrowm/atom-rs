//! The main emulator window: menu bar, the Atom's screen and a status bar.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, TextureHandle, TextureOptions, Vec2};

use super::audio::AudioOutput;
use super::debug::{AppState, Command, DebugWindows};
use super::keymap::atom_key_for;
use crate::cli::Options;
use atom_rs::machine::Atom;
use atom_rs::machine::atm::AtmFile;
use atom_rs::machine::autoplay::TapeAutoplay;
use atom_rs::machine::debug::StopReason;
use atom_rs::machine::disk::DiskImage;
use atom_rs::machine::keyboard::find_key;
use atom_rs::machine::memory::RomSlot;
use atom_rs::machine::tape::TapeImage;
use atom_rs::machine::typist::Typist;
use atom_rs::machine::vdg6847::CYCLES_PER_LINE;
use atom_rs::machine::video::{HEIGHT, WIDTH};

/// The Atom's CPU clock.
const CLOCK_HZ: f64 = 1_000_000.0;

/// If the window stalls (dragged, minimised...), don't try to catch up with
/// more than this much emulated time.
const MAX_CATCH_UP_SECONDS: f64 = 0.1;

/// Emulation gets at most this much real time per screen refresh, leaving the
/// rest for drawing. At high speed settings the Atom simply runs slower than
/// asked if the computer cannot keep up.
const MAX_EMULATION_TIME_PER_UPDATE: Duration = Duration::from_millis(12);

/// Run the CPU in slices this long between checks of the real-time budget.
const SLICE_CYCLES: u64 = 20_000;

const MAX_SPEED: f32 = 100.0;

/// Frames the Atom needs after power-on before it shows its `>` prompt;
/// text from `--run` is typed after this.
const BOOT_FRAMES: u64 = 100;

/// "Run one frame" in the debugger (NTSC: 262 lines).
const CYCLES_PER_FRAME: u64 = 262 * CYCLES_PER_LINE as u64;

pub struct AtomApp {
    atom: Atom,
    rom_dir: PathBuf,
    /// Set if the ROMs could not be loaded; shown instead of the screen.
    rom_error: Option<String>,

    running: bool,
    /// Why the last run stopped (breakpoint, watchpoint...), if it did.
    stop_message: Option<String>,
    debug_windows: DebugWindows,
    /// 1.0 = real Atom speed.
    speed: f32,
    integer_scaling: bool,
    /// Run at maximum speed while the tape is playing.
    fast_tape: bool,

    audio: Option<AudioOutput>,
    muted: bool,
    volume: f32,
    /// A short message for the status bar (file loaded, error, ...).
    notice: Option<String>,
    /// Where the last `.atm` file loaded says to start it.
    loaded_exec: Option<u16>,
    /// Types commands for us (from `--run`, or the tape auto-play).
    typist: Typist,
    /// Start the tape by itself when the Atom says PLAY TAPE.
    autoplay_tape: bool,
    autoplay: TapeAutoplay,
    /// The in-memory quick-save slot (F6 saves, F7 restores).
    quick_state: Option<Vec<u8>>,
    /// The file each disk was loaded from, so Save can write it back.
    disk_paths: [Option<PathBuf>; 2],
    last_update: Instant,
    /// Emulated cycles we are owed but have not run yet.
    cycle_debt: f64,
    /// Speed actually achieved, as a multiple of a real Atom (updated about once a second).
    achieved_speed: f64,
    speed_sample_start: Instant,
    speed_sample_cycles: u64,

    texture: Option<TextureHandle>,
}

impl AtomApp {
    pub fn new(rom_dir: PathBuf, options: Options) -> Self {
        let mut app = AtomApp {
            atom: Atom::new(),
            rom_dir,
            rom_error: None,
            running: true,
            stop_message: None,
            debug_windows: DebugWindows::default(),
            speed: 1.0,
            integer_scaling: true,
            fast_tape: false,
            audio: None,
            muted: false,
            volume: 0.2,
            notice: None,
            loaded_exec: None,
            disk_paths: [None, None],
            quick_state: None,
            typist: Typist::new(),
            autoplay_tape: false,
            autoplay: TapeAutoplay::default(),
            last_update: Instant::now(),
            cycle_debt: 0.0,
            achieved_speed: 0.0,
            speed_sample_start: Instant::now(),
            speed_sample_cycles: 0,
            texture: None,
        };
        app.load_roms();
        app.start_audio();
        let windows = &mut app.debug_windows;
        windows.show_cpu = options.debug;
        windows.show_disassembly = options.debug;
        windows.show_memory = options.debug;
        windows.show_breakpoints = options.debug;
        windows.show_hardware = options.debug;
        app.apply_start_options(&options);
        app
    }

    /// Insert the disks and tape, restore the state and queue the commands
    /// asked for on the command line.
    fn apply_start_options(&mut self, options: &Options) {
        if self.rom_error.is_some() {
            return;
        }
        for (drive, path) in options.disks.iter().enumerate() {
            if let Some(path) = path {
                self.insert_disk_file(drive, path);
            }
        }
        if let Some(path) = &options.tape {
            self.insert_tape_file(path);
            self.autoplay_tape = true;
        }
        self.fast_tape = options.fast_tape;
        let mut start_delay = BOOT_FRAMES;
        if let Some(path) = &options.state {
            self.load_state_path(path);
            start_delay = 0; // a restored machine is already running
        }
        if let Some(text) = &options.run {
            // The disk system must be switched on before it can run anything.
            let uses_disk = options.disks.iter().any(Option::is_some);
            let text = if uses_disk && !text.starts_with("*DOS") {
                format!("*DOS\n{text}")
            } else {
                text.clone()
            };
            let start = self.atom.bus.vdg.frames + start_delay;
            self.typist.type_text(&text, start);
        }
    }

    fn start_audio(&mut self) {
        match AudioOutput::new() {
            Ok(audio) => {
                let sound = &mut self.atom.bus.sound;
                sound.set_sample_rate(CLOCK_HZ, audio.sample_rate() as f64);
                sound.enabled = true;
                self.audio = Some(audio);
            }
            Err(message) => self.notice = Some(format!("No sound: {message}")),
        }
    }

    fn load_roms(&mut self) {
        match self.atom.load_roms(&self.rom_dir) {
            Ok(()) => {
                self.rom_error = None;
                self.atom.reset();
            }
            Err(message) => self.rom_error = Some(message),
        }
    }

    fn choose_rom_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("Folder containing the Atom ROMs")
            .pick_folder()
        {
            self.rom_dir = dir;
            self.load_roms();
        }
    }

    fn load_single_rom(&mut self, slot: RomSlot) {
        let picked = rfd::FileDialog::new()
            .set_title(format!("Load {slot:?} ROM (4096 bytes)"))
            .add_filter("ROM images", &["rom", "bin"])
            .pick_file();
        let Some(path) = picked else { return };
        let result = std::fs::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|data| self.atom.bus.memory.load_rom(slot, &data));
        match result {
            Ok(()) => self.atom.reset(),
            Err(message) => self.rom_error = Some(message),
        }
    }

    /// Copy the host keyboard state onto the Atom's keyboard matrix.
    fn update_keyboard(&mut self, ctx: &egui::Context) {
        let (keys_down, modifiers, break_pressed, quick_save, quick_load) = ctx.input(|i| {
            (
                i.keys_down.clone(),
                i.modifiers,
                i.key_pressed(Key::F12),
                i.key_pressed(Key::F6),
                i.key_pressed(Key::F7),
            )
        });

        let frame = self.atom.bus.vdg.frames;
        let ready = self.atom.ready_for_input();
        let keyboard = &mut self.atom.bus.ppi.keyboard;
        keyboard.release_all();
        let mut atom_shift = modifiers.shift;
        for host_key in keys_down {
            if let Some(press) = atom_key_for(host_key) {
                let (row, column) = find_key(press.key).expect("keymap only names real keys");
                keyboard.set_key(row, column, true);
                atom_shift |= press.shift;
            }
        }
        keyboard.shift = atom_shift;
        keyboard.ctrl = modifiers.ctrl;
        keyboard.rept = modifiers.alt;
        // The typist adds its own key presses on top of the real keyboard's.
        self.typist.update(keyboard, frame, ready);

        // F12 is the BREAK key, which resets the machine.
        if break_pressed {
            self.atom.reset();
        }
        if quick_save {
            self.quick_save();
        }
        if quick_load {
            self.quick_load();
        }
    }

    /// Run the emulated Atom for as long as real time says has passed.
    fn run_emulation(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;
        if self.autoplay_tape {
            self.autoplay.update(&mut self.atom, &mut self.typist);
        }
        if !self.running || self.rom_error.is_some() {
            // Single-stepping in the debugger still makes speaker samples;
            // throw them away so they do not play as a burst on resume.
            self.atom.bus.sound.take_samples();
            return;
        }

        let speed = self.effective_speed();
        self.cycle_debt += elapsed.min(MAX_CATCH_UP_SECONDS) * CLOCK_HZ * speed as f64;

        // Run in slices until we have caught up or used up this update's time budget.
        while self.cycle_debt >= 1.0 && now.elapsed() < MAX_EMULATION_TIME_PER_UPDATE {
            let slice = (self.cycle_debt as u64).min(SLICE_CYCLES);
            let start = self.atom.bus.cycles;
            let stop = self.atom.run_cycles(slice);
            let executed = self.atom.bus.cycles - start;
            self.cycle_debt -= executed as f64;
            self.speed_sample_cycles += executed;
            if stop.is_some() {
                self.note_stop(stop);
                self.cycle_debt = 0.0;
                break;
            }
        }
        // Out of time: forget the backlog rather than spiralling.
        if self.cycle_debt >= 1.0 {
            self.cycle_debt = 0.0;
        }
        self.update_achieved_speed();
        self.send_audio(speed);
    }

    /// The speed we are really running at: the slider, unless fast-tape is
    /// on and the tape is playing.
    fn effective_speed(&self) -> f32 {
        if self.fast_tape && self.atom.bus.tape.playing {
            MAX_SPEED
        } else {
            self.speed
        }
    }

    /// Pass the speaker sound produced since last time to the sound card.
    /// Sound is only played at normal speed or slower; faster would just be
    /// a squeal.
    fn send_audio(&mut self, speed: f32) {
        let samples = self.atom.bus.sound.take_samples();
        let Some(audio) = &self.audio else { return };
        if self.muted || speed > 1.05 {
            return;
        }
        // The samples are 0.0-1.0 (the fraction of time the speaker was high);
        // centre them on zero and apply the volume.
        let gain = self.volume * 2.0;
        let centred: Vec<f32> = samples.iter().map(|s| (s - 0.5) * gain).collect();
        audio.push(&centred);
    }

    // ---- Tape and file loading ------------------------------------------

    fn insert_tape(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Insert tape")
            .add_filter("UEF tape images", &["uef"])
            .pick_file();
        if let Some(path) = picked {
            self.insert_tape_file(&path);
        }
    }

    fn insert_tape_file(&mut self, path: &Path) {
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|data| TapeImage::from_uef(&data));
        self.notice = Some(match result {
            Ok(image) => {
                self.atom.bus.tape.insert(image);
                format!(
                    "Inserted {}. Use Tape > Play when the Atom says PLAY TAPE.",
                    path.display()
                )
            }
            Err(message) => format!("Cannot load {}: {message}", path.display()),
        });
    }

    fn save_tape(&mut self) {
        let Some(image) = self.atom.bus.tape.image() else {
            return;
        };
        let data = image.to_uef();
        let picked = rfd::FileDialog::new()
            .set_title("Save tape")
            .add_filter("UEF tape images", &["uef"])
            .set_file_name("tape.uef")
            .save_file();
        let Some(path) = picked else { return };
        self.notice = Some(match std::fs::write(&path, data) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Cannot save {}: {e}", path.display()),
        });
    }

    /// Start recording what the Atom writes to the cassette, or stop and put
    /// the recording into the player (ready to rewind, play or save).
    fn toggle_recording(&mut self) {
        let tape = &mut self.atom.bus.tape;
        if tape.recording {
            let seconds = tape.recorded_seconds();
            let image = tape.finish_recording();
            tape.insert(image);
            self.notice = Some(format!(
                "Recorded {seconds:.1}s. Tape > Save tape to keep it."
            ));
        } else {
            tape.recording = true;
        }
    }

    // ---- Save states -----------------------------------------------------

    fn quick_save(&mut self) {
        self.notice = Some(match self.atom.save_state() {
            Ok(state) => {
                self.quick_state = Some(state);
                "Quick-saved (F7 to restore)".to_string()
            }
            Err(message) => format!("Cannot save state: {message}"),
        });
    }

    fn quick_load(&mut self) {
        let Some(state) = self.quick_state.clone() else {
            self.notice = Some("Nothing quick-saved yet (F6 saves)".to_string());
            return;
        };
        self.restore_state(&state, "Quick-saved state restored");
    }

    fn restore_state(&mut self, state: &[u8], success_message: &str) {
        self.notice = Some(match self.atom.load_state(state) {
            Ok(()) => {
                self.cycle_debt = 0.0;
                self.stop_message = None;
                success_message.to_string()
            }
            Err(message) => format!("Cannot load state: {message}"),
        });
    }

    fn save_state_file(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Save state")
            .add_filter("Atom saved states", &["atomstate"])
            .set_file_name("atom.atomstate")
            .save_file();
        let Some(path) = picked else { return };
        self.notice = Some(
            match self
                .atom
                .save_state()
                .and_then(|state| std::fs::write(&path, state).map_err(|e| e.to_string()))
            {
                Ok(()) => format!("Saved state to {}", path.display()),
                Err(message) => format!("Cannot save state: {message}"),
            },
        );
    }

    fn load_state_file(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Load state")
            .add_filter("Atom saved states", &["atomstate"])
            .pick_file();
        if let Some(path) = picked {
            self.load_state_path(&path);
        }
    }

    fn load_state_path(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(state) => self.restore_state(&state, &format!("Restored {}", path.display())),
            Err(e) => self.notice = Some(format!("Cannot read {}: {e}", path.display())),
        }
    }

    // ---- Disks -----------------------------------------------------------

    fn insert_disk(&mut self, drive: usize) {
        let picked = rfd::FileDialog::new()
            .set_title(format!("Insert disk in drive {drive}"))
            .add_filter("Disk images", &["ssd", "dsd"])
            .pick_file();
        if let Some(path) = picked {
            self.insert_disk_file(drive, &path);
        }
    }

    fn insert_disk_file(&mut self, drive: usize, path: &Path) {
        // .dsd files hold two sides; .ssd one.
        let sides = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dsd"))
        {
            2
        } else {
            1
        };
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|data| DiskImage::from_bytes(data, sides));
        self.notice = Some(match result {
            Ok(disk) => {
                self.atom.bus.fdc.drives[drive] = Some(disk);
                self.disk_paths[drive] = Some(path.to_path_buf());
                if self.atom.bus.memory.disk_interface_fitted {
                    format!(
                        "Disk {drive}: {}. Type *DOS to use the disk system.",
                        path.display()
                    )
                } else {
                    "Disk inserted, but there is no disk ROM (dosrom.rom) loaded".to_string()
                }
            }
            Err(message) => format!("Cannot load {}: {message}", path.display()),
        });
    }

    fn new_blank_disk(&mut self, drive: usize) {
        self.atom.bus.fdc.drives[drive] = Some(DiskImage::blank_80_track(1));
        self.disk_paths[drive] = None;
        self.notice = Some(format!(
            "Blank 80 track disk in drive {drive} (not saved yet)"
        ));
    }

    /// Write the disk back to its file, or ask for a file name if it has none
    /// (or `choose_file` is set).
    fn save_disk(&mut self, drive: usize, choose_file: bool) {
        let Some(disk) = &self.atom.bus.fdc.drives[drive] else {
            return;
        };
        let path = match (&self.disk_paths[drive], choose_file) {
            (Some(path), false) => path.clone(),
            _ => {
                let extension = if disk.sides == 2 { "dsd" } else { "ssd" };
                let picked = rfd::FileDialog::new()
                    .set_title(format!("Save disk {drive}"))
                    .add_filter("Disk images", &[extension])
                    .set_file_name(format!("disk{drive}.{extension}"))
                    .save_file();
                match picked {
                    Some(path) => path,
                    None => return,
                }
            }
        };
        self.notice = Some(match std::fs::write(&path, disk.to_bytes()) {
            Ok(()) => {
                if let Some(disk) = &mut self.atom.bus.fdc.drives[drive] {
                    disk.modified = false;
                }
                self.disk_paths[drive] = Some(path.clone());
                format!("Saved {}", path.display())
            }
            Err(e) => format!("Cannot save {}: {e}", path.display()),
        });
    }

    fn eject_disk(&mut self, drive: usize) {
        self.atom.bus.fdc.drives[drive] = None;
        self.disk_paths[drive] = None;
    }

    /// Copy an `.atm` program into RAM.
    fn load_atm(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Load .atm file into memory")
            .add_filter("Atom programs", &["atm"])
            .pick_file();
        let Some(path) = picked else { return };
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|data| AtmFile::parse(&data))
            .and_then(|file| self.atom.load_atm(&file).map(|()| file));
        self.notice = Some(match result {
            Ok(file) => {
                self.loaded_exec = Some(file.exec_address);
                format!(
                    "Loaded {} ({} bytes at ${:04X}). Start it from File > Run loaded program.",
                    file.name,
                    file.data.len(),
                    file.load_address
                )
            }
            Err(message) => format!("Cannot load {}: {message}", path.display()),
        });
    }

    // ---- Debugger commands ---------------------------------------------

    fn execute(&mut self, command: Command) {
        match command {
            Command::Run => self.resume(),
            Command::Pause => {
                self.running = false;
                self.atom.bus.debug.target = None;
            }
            Command::Step => self.step_once(),
            Command::StepOver => match self.atom.step_over_target() {
                Some(target) => {
                    self.atom.bus.debug.target = Some(target);
                    self.resume();
                }
                None => self.step_once(),
            },
            Command::StepFrame => {
                self.running = false;
                self.stop_message = None;
                self.atom.bus.debug.skip_breakpoint_once = true;
                let stop = self.atom.run_cycles(CYCLES_PER_FRAME);
                self.note_stop(stop);
            }
            Command::RunTo(addr) => {
                self.atom.bus.debug.target = Some(addr);
                self.resume();
            }
            Command::Reset => self.atom.reset(),
            Command::Nmi => self.atom.bus.nmi_edge = true,
            Command::ToggleIrq => self.atom.bus.external_irq = !self.atom.bus.external_irq,
        }
    }

    /// Start running. The first instruction runs even if it sits on a
    /// breakpoint, otherwise you could never continue from one.
    fn resume(&mut self) {
        self.running = true;
        self.stop_message = None;
        self.atom.bus.debug.skip_breakpoint_once = true;
    }

    fn step_once(&mut self) {
        self.running = false;
        self.stop_message = None;
        self.atom.step();
        let stop = self.atom.bus.debug.stop.take();
        self.note_stop(stop);
    }

    fn note_stop(&mut self, reason: Option<StopReason>) {
        if let Some(reason) = reason {
            self.running = false;
            self.stop_message = Some(reason.to_string());
        }
    }

    fn update_achieved_speed(&mut self) {
        let seconds = self.speed_sample_start.elapsed().as_secs_f64();
        if seconds >= 1.0 {
            self.achieved_speed = self.speed_sample_cycles as f64 / seconds / CLOCK_HZ;
            self.speed_sample_start = Instant::now();
            self.speed_sample_cycles = 0;
        }
    }

    fn upload_screen(&mut self, ctx: &egui::Context) -> egui::TextureId {
        let rgba = self.atom.bus.framebuffer.to_rgba();
        let image = egui::ColorImage::from_rgba_unmultiplied([WIDTH, HEIGHT], &rgba);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::NEAREST),
            None => {
                self.texture = Some(ctx.load_texture("atom-screen", image, TextureOptions::NEAREST))
            }
        }
        self.texture.as_ref().expect("just created").id()
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.menu_button("Load ROM", |ui| {
                    for slot in RomSlot::ALL {
                        if ui.button(format!("{slot:?}...")).clicked() {
                            self.load_single_rom(slot);
                            ui.close();
                        }
                    }
                });
                if ui.button("Choose ROM folder...").clicked() {
                    self.choose_rom_folder();
                    ui.close();
                }
                ui.separator();
                if ui.button("Save state...").clicked() {
                    self.save_state_file();
                    ui.close();
                }
                if ui.button("Load state...").clicked() {
                    self.load_state_file();
                    ui.close();
                }
                if ui.button("Quick save (F6)").clicked() {
                    self.quick_save();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.quick_state.is_some(),
                        egui::Button::new("Quick load (F7)"),
                    )
                    .clicked()
                {
                    self.quick_load();
                    ui.close();
                }
                ui.separator();
                if ui.button("Load .atm file into memory...").clicked() {
                    self.load_atm();
                    ui.close();
                }
                let run_label = match self.loaded_exec {
                    Some(addr) => format!("Run loaded program (jump to ${addr:04X})"),
                    None => "Run loaded program".to_string(),
                };
                if ui
                    .add_enabled(self.loaded_exec.is_some(), egui::Button::new(run_label))
                    .clicked()
                {
                    self.atom.cpu.regs.pc = self.loaded_exec.unwrap_or(self.atom.cpu.regs.pc);
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Machine", |ui| {
                if ui.button("Reset (F12)").clicked() {
                    self.execute(Command::Reset);
                    ui.close();
                }
                let (label, command) = if self.running {
                    ("Pause", Command::Pause)
                } else {
                    ("Resume", Command::Run)
                };
                if ui.button(label).clicked() {
                    self.execute(command);
                    ui.close();
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.speed, 0.1..=MAX_SPEED)
                            .logarithmic(true)
                            .text("speed"),
                    );
                    if ui.button("1x").clicked() {
                        self.speed = 1.0;
                    }
                });
            });
            ui.menu_button("Tape", |ui| {
                if ui.button("Insert tape (UEF)...").clicked() {
                    self.insert_tape();
                    ui.close();
                }
                let has_tape = self.atom.bus.tape.has_tape();
                ui.add_enabled_ui(has_tape, |ui| {
                    let playing = self.atom.bus.tape.playing;
                    if ui.button(if playing { "Stop" } else { "Play" }).clicked() {
                        self.atom.bus.tape.playing = !playing;
                        ui.close();
                    }
                    if ui.button("Rewind").clicked() {
                        self.atom.bus.tape.rewind();
                        ui.close();
                    }
                    if ui.button("Eject").clicked() {
                        self.atom.bus.tape.eject();
                        ui.close();
                    }
                    if ui.button("Save tape as UEF...").clicked() {
                        self.save_tape();
                        ui.close();
                    }
                });
                ui.separator();
                let record_label = if self.atom.bus.tape.recording {
                    "Stop recording"
                } else {
                    "Record"
                };
                if ui.button(record_label).clicked() {
                    self.toggle_recording();
                    ui.close();
                }
                ui.separator();
                ui.checkbox(
                    &mut self.fast_tape,
                    "Fast tape (maximum speed while playing)",
                );
            });
            ui.menu_button("Disk", |ui| {
                if !self.atom.bus.memory.disk_interface_fitted {
                    ui.label("No disk ROM loaded (dosrom.rom)");
                }
                for drive in 0..2 {
                    let disk = &self.atom.bus.fdc.drives[drive];
                    let (present, modified) =
                        (disk.is_some(), disk.as_ref().is_some_and(|d| d.modified));
                    let title = match (present, modified) {
                        (false, _) => format!("Drive {drive}: empty"),
                        (true, false) => format!("Drive {drive}"),
                        (true, true) => format!("Drive {drive} (unsaved changes)"),
                    };
                    ui.menu_button(title, |ui| {
                        if ui.button("Insert disk image...").clicked() {
                            self.insert_disk(drive);
                            ui.close();
                        }
                        if ui.button("New blank disk").clicked() {
                            self.new_blank_disk(drive);
                            ui.close();
                        }
                        ui.add_enabled_ui(present, |ui| {
                            if ui.button("Save").clicked() {
                                self.save_disk(drive, false);
                                ui.close();
                            }
                            if ui.button("Save as...").clicked() {
                                self.save_disk(drive, true);
                                ui.close();
                            }
                            if ui.button("Eject").clicked() {
                                self.eject_disk(drive);
                                ui.close();
                            }
                        });
                    });
                }
            });
            ui.menu_button("Sound", |ui| {
                if self.audio.is_none() {
                    ui.label("No audio device available");
                }
                ui.checkbox(&mut self.muted, "Mute");
                ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text("volume"));
                ui.label("Sound plays at normal speed or slower.");
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.integer_scaling, "Integer scaling");
            });
            ui.menu_button("Debug", |ui| {
                let windows = &mut self.debug_windows;
                ui.checkbox(&mut windows.show_cpu, "CPU and controls");
                ui.checkbox(&mut windows.show_disassembly, "Disassembly");
                ui.checkbox(&mut windows.show_memory, "Memory");
                ui.checkbox(&mut windows.show_breakpoints, "Breakpoints");
                ui.checkbox(&mut windows.show_hardware, "Hardware");
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let regs = self.atom.cpu.regs.clone();
        ui.horizontal(|ui| {
            ui.label(if self.running { "Running" } else { "Paused" });
            if let Some(message) = &self.stop_message {
                ui.colored_label(egui::Color32::LIGHT_RED, message);
            }
            if self.typist.is_busy() {
                // Make the automatic typing visible: waiting for the Atom to
                // start up, or how much is left to type.
                let frames = self.atom.bus.vdg.frames;
                let text = if frames < self.typist.start_frame() {
                    "Auto-typing soon...".to_string()
                } else {
                    format!("Auto-typing ({} keys left)", self.typist.remaining())
                };
                ui.separator();
                ui.colored_label(egui::Color32::LIGHT_GREEN, text);
            }
            let tape = &self.atom.bus.tape;
            if tape.recording {
                ui.separator();
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("REC {:.1}s", tape.recorded_seconds()),
                );
            } else if tape.has_tape() {
                ui.separator();
                let state = if tape.playing { "playing" } else { "stopped" };
                ui.label(format!("Tape {state} {:.0}%", tape.progress() * 100.0));
            }
            for (drive, disk) in self.atom.bus.fdc.drives.iter().enumerate() {
                if let Some(disk) = disk {
                    ui.separator();
                    let mark = if disk.modified { "*" } else { "" };
                    ui.label(format!("D{drive}{mark}"));
                }
            }
            if let Some(notice) = &self.notice {
                ui.separator();
                ui.label(notice);
            }
            ui.separator();
            ui.label(format!(
                "{:.1}x (actual {:.1}x)",
                self.speed, self.achieved_speed
            ));
            // One click back to real Atom speed.
            if ui
                .add_enabled(self.speed != 1.0, egui::Button::new("1x").small())
                .on_hover_text("Back to real Atom speed")
                .clicked()
            {
                self.speed = 1.0;
            }
            ui.separator();
            ui.monospace(format!(
                "PC={:04X} A={:02X} X={:02X} Y={:02X} SP={:02X} P={:02X}",
                regs.pc, regs.a, regs.x, regs.y, regs.sp, regs.p
            ));
            ui.separator();
            ui.label(format!("frame {}", self.atom.bus.vdg.frames));
        });
    }

    fn screen(&mut self, ui: &mut egui::Ui) {
        if let Some(message) = self.rom_error.clone() {
            ui.vertical_centered(|ui| {
                ui.heading("Cannot start the Atom");
                ui.label(message);
                ui.label("Put akernel.rom, abasic.rom and afloat.rom in the roms/ folder, or:");
                if ui.button("Choose ROM folder...").clicked() {
                    self.choose_rom_folder();
                }
            });
            return;
        }

        let texture = self.upload_screen(ui.ctx());
        let available = ui.available_size();
        let mut scale = (available.x / WIDTH as f32).min(available.y / HEIGHT as f32);
        if self.integer_scaling && scale >= 1.0 {
            scale = scale.floor();
        }
        let size = Vec2::new(WIDTH as f32, HEIGHT as f32) * scale;
        ui.centered_and_justified(|ui| {
            ui.add(egui::Image::new((texture, size)));
        });
    }
}

impl eframe::App for AtomApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_keyboard(ctx);
        self.run_emulation();
        ctx.request_repaint(); // keep emulating even when nothing else changes
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| self.screen(ui));

        // Debugger windows are separate OS windows; they hand back commands.
        let state = AppState {
            running: self.running,
            stop_message: self.stop_message.as_deref(),
        };
        let commands = self.debug_windows.show(ui.ctx(), &mut self.atom, &state);
        for command in commands {
            self.execute(command);
        }
    }
}
