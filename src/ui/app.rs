//! The main emulator window: menu bar, the Atom's screen and a status bar.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, TextureHandle, TextureOptions, Vec2};

use super::archive_job::{ArchiveJob, Progress};
use super::audio::AudioOutput;
use super::debug::{Command, DebugWindows};
use super::help_window::HelpWindows;
use super::keyboard_window::KeyboardWindow;
use super::keymap::KeyMap;
use super::logic::{self, CycleDebt};
use super::screenshot;
use super::settings::Settings;
use super::tape_window::TapeCatalogueWindow;
use crate::cli::Options;
use atom_rs::machine::Atom;
use atom_rs::machine::atm::AtmFile;
use atom_rs::machine::atom::Machine;
use atom_rs::machine::autoplay::TapeAutoplay;
use atom_rs::machine::debug_session::DebugSession;
use atom_rs::machine::disk::DiskImage;
use atom_rs::machine::memory::{GDOS_BANKS, MAX_VIDEO_RAM_KB, RamPreset, RomSlot, SidAddress};
use atom_rs::machine::sid::SidModel;
use atom_rs::machine::tape::TapeImage;
use atom_rs::machine::typist::Typist;

/// Emulation gets at most this much real time per screen refresh, leaving the
/// rest for drawing. At high speed settings the Atom simply runs slower than
/// asked if the computer cannot keep up.
const MAX_EMULATION_TIME_PER_UPDATE: Duration = Duration::from_millis(12);

/// How long sound stays off after the Atom restarts, so the fade out and the
/// fade back in do not overlap the restart itself.
const RESTART_AUDIO_HOLD: Duration = Duration::from_millis(150);

/// Frames the Atom needs after power-on before it shows its `>` prompt;
/// text from `--run` is typed after this.
const BOOT_FRAMES: u64 = 100;

pub struct AtomApp {
    atom: Atom,
    rom_dir: PathBuf,
    /// Set if the ROMs could not be loaded; shown instead of the screen.
    rom_error: Option<String>,

    /// Running or paused, and why the last run stopped (see [`DebugSession`]).
    session: DebugSession,
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
    /// Which host keys press which Atom keys, and the window that edits it.
    keymap: KeyMap,
    /// Do the arrow keys and Space act as a joystick (Input menu)?
    joystick_keys: bool,
    fullscreen: bool,
    /// After a restart, sound stays off until this time, so the change in the
    /// speaker's level does not click (see [`AtomApp::restart_audio`]).
    audio_hold_until: Option<Instant>,
    /// The folder last used as an SD card, so switching back to the SD card
    /// machine does not ask again.
    card_folder: Option<PathBuf>,
    /// The standard archive being fetched in the background, so the SD card
    /// machine can start with it (see [`AtomApp::switch_machine`]).
    archive_job: Option<ArchiveJob>,
    /// Is the colour board fitted? (Without it the picture is grey.)
    colour_board: bool,
    keyboard_window: KeyboardWindow,
    help: HelpWindows,
    tape_catalogue: TapeCatalogueWindow,
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
    cycle_debt: CycleDebt,
    /// Speed actually achieved, as a multiple of a real Atom (updated about once a second).
    achieved_speed: f64,
    speed_sample_start: Instant,
    speed_sample_cycles: u64,

    texture: Option<TextureHandle>,
    /// The settings as last written to disk (see [`AtomApp::save_settings_if_changed`]).
    saved_settings: Settings,
}

impl AtomApp {
    pub fn new(rom_dir: PathBuf, options: Options) -> Self {
        let mut app = AtomApp {
            atom: Atom::new(),
            rom_dir,
            rom_error: None,
            session: DebugSession::default(),
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
            keymap: KeyMap::default(),
            joystick_keys: false,
            fullscreen: false,
            audio_hold_until: None,
            card_folder: None,
            archive_job: None,
            colour_board: true,
            keyboard_window: KeyboardWindow::default(),
            help: HelpWindows::default(),
            tape_catalogue: TapeCatalogueWindow::default(),
            typist: Typist::new(),
            autoplay_tape: false,
            autoplay: TapeAutoplay::default(),
            last_update: Instant::now(),
            cycle_debt: CycleDebt::default(),
            achieved_speed: 0.0,
            speed_sample_start: Instant::now(),
            speed_sample_cycles: 0,
            texture: None,
            saved_settings: Settings::default(),
        };
        app.load_roms();
        app.start_audio();
        let windows = &mut app.debug_windows;
        windows.show_cpu = options.debug;
        windows.show_disassembly = options.debug;
        windows.show_memory = options.debug;
        windows.show_breakpoints = options.debug;
        windows.show_hardware = options.debug;
        windows.show_console = options.debug;
        let (keymap, problem) = KeyMap::load();
        app.keymap = keymap;
        if let Some(message) = problem {
            app.notice = Some(message);
        }
        let (settings, problem) = Settings::load();
        if let Some(message) = problem {
            app.notice = Some(message);
        }
        // What the command line asks for (a card folder, a saved state) decides
        // the machine, so only restore the saved one when it asks for nothing.
        let restore_machine = options.sdcard.is_none() && options.state.is_none();
        app.apply_settings(&settings, restore_machine);
        app.saved_settings = app.settings();
        app.apply_start_options(&options);
        app
    }

    // ---- Remembered settings ------------------------------------------------------

    /// The settings as they are now.
    fn settings(&self) -> Settings {
        let bus = &self.atom.bus;
        Settings {
            machine: self.atom.machine(),
            card_folder: bus
                .atommc
                .card_folder()
                .map(Path::to_path_buf)
                .or_else(|| self.card_folder.clone()),
            ram_preset: bus.memory.ram_preset(),
            video_ram_kb: bus.memory.video_ram_kb(),
            colour_board: self.colour_board,
            show_border: bus.framebuffer.show_border,
            snow: bus.snow,
            sid_fitted: bus.memory.sid_fitted,
            sid_address: bus.memory.sid_address,
            sid_model: bus.sid.model(),
            volume: self.volume,
            muted: self.muted,
            integer_scaling: self.integer_scaling,
            fast_tape: self.fast_tape,
            joystick_keys: self.joystick_keys,
        }
    }

    /// Put the saved settings into effect. The machine goes first, because
    /// choosing one restarts the Atom and sets up its own hardware.
    fn apply_settings(&mut self, settings: &Settings, restore_machine: bool) {
        self.card_folder = settings.card_folder.clone();
        if restore_machine && settings.machine != Machine::FloppyAtom {
            let result = self.atom.set_machine(
                settings.machine,
                &self.rom_dir,
                settings.card_folder.as_deref(),
            );
            if let Err(message) = result {
                self.notice = Some(format!("Could not restore the saved machine: {message}"));
            }
        }
        // RAM size is the one setting the RAM/ROM board makes meaningless.
        if self.atom.machine() != Machine::SdCard && self.atom.machine() != Machine::BbcBasic {
            let _ = self.atom.set_ram_preset(settings.ram_preset);
        }
        let bus = &mut self.atom.bus;
        bus.memory.set_video_ram_kb(settings.video_ram_kb);
        bus.snow = settings.snow;
        bus.framebuffer.show_border = settings.show_border;
        bus.memory.sid_fitted = settings.sid_fitted;
        bus.memory.sid_address = settings.sid_address;
        bus.sid.set_model(settings.sid_model);
        self.colour_board = settings.colour_board;
        bus.framebuffer.monochrome = !settings.colour_board;
        self.volume = settings.volume;
        self.muted = settings.muted;
        self.integer_scaling = settings.integer_scaling;
        self.fast_tape = settings.fast_tape;
        self.joystick_keys = settings.joystick_keys;
    }

    /// Write the settings out when they have changed. Not while a mouse
    /// button is down, so dragging the volume slider does not write a file on
    /// every frame; it is saved when the button is let go.
    fn save_settings_if_changed(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let now = self.settings();
        if now != self.saved_settings {
            if let Err(message) = now.save() {
                self.notice = Some(format!("Cannot save the settings: {message}"));
            }
            self.saved_settings = now; // even on failure, so it does not retry every frame
        }
    }

    /// Insert the disks and tape, restore the state and queue the commands
    /// asked for on the command line.
    fn apply_start_options(&mut self, options: &Options) {
        if self.rom_error.is_some() {
            return;
        }
        // A flag can only switch these on; otherwise the saved setting stands.
        self.joystick_keys |= options.joystick;
        if let Some(folder) = &options.sdcard {
            self.use_sd_card(folder.clone());
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
        self.fast_tape |= options.fast_tape;
        let mut start_delay = BOOT_FRAMES;
        if let Some(path) = &options.state {
            self.load_state_path(path);
            start_delay = 0; // a restored machine is already running
        }
        if let Some(text) = &options.run {
            let uses_disk = options.disks.iter().any(Option::is_some);
            let text = logic::text_to_run(text, uses_disk, options.sdcard.is_some());
            let start = self.atom.bus.vdg.frames + start_delay;
            self.typist.type_text(&text, start);
        }
    }

    fn start_audio(&mut self) {
        // The tests build whole apps; they must not grab (or complain about the
        // lack of) a sound card.
        if cfg!(test) {
            return;
        }
        match AudioOutput::new() {
            Ok(audio) => {
                self.atom.bus.set_audio_sample_rate(audio.sample_rate());
                self.atom.bus.sound.enabled = true;
                self.audio = Some(audio);
            }
            Err(message) => self.notice = Some(format!("No sound: {message}")),
        }
    }

    /// Call after anything that restarts the Atom. The speaker and the SID
    /// change level instantly, and the sound card would play that jump as a
    /// click. Instead the queued sound is dropped, which lets the output fade
    /// out, and no new sound is sent for a moment, so it fades back in.
    fn restart_audio(&mut self) {
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        self.audio_hold_until = Some(Instant::now() + RESTART_AUDIO_HOLD);
    }

    /// Save the picture on the screen as a PNG. With `choose_file` a dialog asks
    /// where; otherwise it goes in the current folder with a dated name.
    fn save_screenshot(&mut self, choose_file: bool) {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let name = screenshot::file_name(seconds);
        let path = if choose_file {
            let picked = rfd::FileDialog::new()
                .set_title("Save screenshot")
                .set_file_name(&name)
                .add_filter("PNG picture", &["png"])
                .save_file();
            let Some(path) = picked else { return };
            path
        } else {
            PathBuf::from(&name)
        };
        let (width, height) = self.atom.bus.framebuffer.shown_size();
        let rgba = self.atom.bus.framebuffer.to_rgba_shown();
        self.notice = Some(
            match screenshot::encode_png(width as u32, height as u32, &rgba)
                .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
            {
                Ok(()) => format!("Screenshot saved: {}", path.display()),
                Err(message) => format!("Cannot save the screenshot: {message}"),
            },
        );
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        self.fullscreen = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    fn load_roms(&mut self) {
        match self.atom.load_roms(&self.rom_dir) {
            Ok(()) => {
                self.rom_error = None;
                self.atom.reset();
                self.restart_audio();
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

    /// Switch to an Atom with an AtoMMC SD card whose files are in `folder`.
    fn use_sd_card(&mut self, folder: PathBuf) {
        self.typist.cancel();
        self.restart_audio();
        self.card_folder = Some(folder.clone());
        self.notice = Some(match self.atom.use_sd_card(&self.rom_dir, folder.clone()) {
            Ok(()) => format!(
                "SD card: {}. Try *CAT, or *MENU for the software archive.",
                folder.display()
            ),
            Err(message) => format!("Cannot use the SD card: {message}"),
        });
    }

    fn choose_sd_card(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("Folder to use as the SD card")
            .pick_folder()
        {
            self.use_sd_card(folder);
        }
    }

    /// Hardware > Machine: become a different kind of Atom. The SD card machine
    /// needs a folder: the one used last, or else the standard software
    /// archive, which is downloaded first if it is not there yet. (Another
    /// folder can be chosen afterwards, in the same menu.)
    fn switch_machine(&mut self, machine: Machine) {
        // Choosing again while the archive is still coming cancels the wait.
        self.archive_job = None;
        // Typing queued for the old machine (a --run command still going, say)
        // must not carry on into the new one.
        self.typist.cancel();
        self.restart_audio();
        if let Some(folder) = self.atom.bus.atommc.card_folder() {
            self.card_folder = Some(folder.to_path_buf()); // remember it on the way out
        }
        let folder = self.card_folder.clone();
        if machine == Machine::SdCard && folder.is_none() {
            self.notice = Some(
                "Getting the Atom Software Archive (the first time only; about 16 MB)...".into(),
            );
            self.archive_job = Some(ArchiveJob::start_standard());
            return; // the machine changes when it arrives (see `poll_archive_job`)
        }
        match self
            .atom
            .set_machine(machine, &self.rom_dir, folder.as_deref())
        {
            Ok(()) => {
                if machine == Machine::SdCard {
                    self.card_folder = folder;
                }
                self.notice = Some(format!("Now: {}", machine.label()));
            }
            Err(message) => self.notice = Some(format!("Cannot switch: {message}")),
        }
    }

    /// Called every frame: show how the archive download is going, and when
    /// it has arrived, start the SD card machine with it.
    fn poll_archive_job(&mut self) {
        let Some(job) = &mut self.archive_job else {
            return;
        };
        for report in job.poll() {
            match report {
                Progress::Said(message) => self.notice = Some(message),
                Progress::Done(Ok(folder)) => {
                    self.archive_job = None;
                    self.use_sd_card(folder);
                    return;
                }
                Progress::Done(Err(message)) => {
                    self.archive_job = None;
                    self.notice = Some(format!(
                        "Could not get the software archive: {message}. Check your internet \
                         connection, or start with --sdcard FOLDER to use a folder you have."
                    ));
                    return;
                }
            }
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
            Ok(()) => {
                self.atom.reset();
                self.restart_audio();
            }
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
        let stick = logic::press_host_keys(
            keyboard,
            keys_down,
            modifiers,
            &self.keymap,
            self.joystick_keys,
        );
        // The typist adds its own key presses on top of the real keyboard's.
        self.typist.update(keyboard, frame, ready);
        self.atom.set_joystick(self.joystick_keys.then_some(stick));

        // F10 saves a screenshot, F11 toggles full screen (neither is an Atom key).
        let (screenshot_key, fullscreen_key) =
            ctx.input(|i| (i.key_pressed(Key::F10), i.key_pressed(Key::F11)));
        if screenshot_key {
            self.save_screenshot(false);
        }
        if fullscreen_key {
            self.set_fullscreen(ctx, !self.fullscreen);
        }

        // F12 is the BREAK key, which resets the machine.
        if break_pressed {
            self.atom.reset();
            self.restart_audio();
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
        if !self.session.running || self.rom_error.is_some() {
            // Single-stepping in the debugger still makes speaker samples;
            // throw them away so they do not play as a burst on resume.
            self.atom.bus.sound.take_samples();
            return;
        }

        let speed = self.effective_speed();
        self.cycle_debt.owe(elapsed, speed);

        // Run in slices until we have caught up or used up this update's time budget.
        while let Some(slice) = self.cycle_debt.next_slice() {
            if now.elapsed() >= MAX_EMULATION_TIME_PER_UPDATE {
                break;
            }
            let start = self.atom.bus.cycles;
            let stopped = self.session.run_cycles(&mut self.atom, slice);
            let executed = self.atom.bus.cycles - start;
            self.cycle_debt.paid(executed);
            self.speed_sample_cycles += executed;
            if stopped {
                self.cycle_debt.forget();
                break;
            }
        }
        // Out of time: forget the backlog rather than spiralling.
        self.cycle_debt.forget_backlog();
        self.update_achieved_speed();
        self.send_audio(speed);
    }

    /// The speed we are really running at: the slider, unless fast-tape is
    /// on and the tape is playing.
    fn effective_speed(&self) -> f32 {
        logic::effective_speed(self.speed, self.fast_tape, self.atom.bus.tape.playing)
    }

    /// Pass the speaker sound produced since last time to the sound card.
    /// Sound is only played at normal speed or slower; faster would just be
    /// a squeal.
    fn send_audio(&mut self, speed: f32) {
        let samples = self.atom.bus.sound.take_samples();
        let Some(audio) = &self.audio else { return };
        if !logic::sound_is_audible(self.muted, speed) {
            return;
        }
        // Just after a restart: stay silent for a moment (see restart_audio).
        if let Some(until) = self.audio_hold_until {
            if Instant::now() < until {
                return;
            }
            self.audio_hold_until = None;
        }
        // The samples are 0.0-1.0 (the fraction of time the speaker was high);
        // centre them on zero and apply the volume.
        audio.push(&logic::centred_samples(&samples, self.volume));
    }

    // ---- Tape and file loading ------------------------------------------

    fn insert_tape(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Insert tape")
            .add_filter("Tape images (UEF or CSW)", &["uef", "csw"])
            .pick_file();
        if let Some(path) = picked {
            self.insert_tape_file(&path);
        }
    }

    fn insert_tape_file(&mut self, path: &Path) {
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|data| TapeImage::from_file(&data));
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
        self.restart_audio();
        self.notice = Some(match self.atom.load_state(state) {
            Ok(()) => {
                self.cycle_debt.forget();
                self.session.stop_message = None;
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
        self.session.execute(&mut self.atom, command);
        if command == Command::Reset {
            self.restart_audio();
        }
    }

    fn update_achieved_speed(&mut self) {
        let seconds = self.speed_sample_start.elapsed().as_secs_f64();
        if seconds >= 1.0 {
            self.achieved_speed = logic::achieved_speed(self.speed_sample_cycles, seconds);
            self.speed_sample_start = Instant::now();
            self.speed_sample_cycles = 0;
        }
    }

    fn upload_screen(&mut self, ctx: &egui::Context) -> egui::TextureId {
        let (width, height) = self.atom.bus.framebuffer.shown_size();
        let rgba = self.atom.bus.framebuffer.to_rgba_shown();
        let image = egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
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
                if ui
                    .button("Save screenshot... (F10 saves at once)")
                    .clicked()
                {
                    self.save_screenshot(true);
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
            ui.menu_button("Run", |ui| {
                if ui.button("Reset (F12)").clicked() {
                    self.execute(Command::Reset);
                    ui.close();
                }
                let (label, command) = if self.session.running {
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
                        egui::Slider::new(&mut self.speed, 0.1..=logic::MAX_SPEED)
                            .logarithmic(true)
                            .text("speed"),
                    );
                    if ui.button("1x").clicked() {
                        self.speed = 1.0;
                    }
                });
            });
            ui.menu_button("Tape", |ui| {
                if ui.button("Insert tape (UEF or CSW)...").clicked() {
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
                    if ui.button("Catalogue...").clicked() {
                        self.tape_catalogue.open = true;
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
                // The two floppy machines have drives, but only if there is a
                // disk system to drive them: Atom DOS (dosrom.rom) or GDOS.
                let machine = self.atom.machine();
                let drives_work = match machine {
                    Machine::FloppyAtom => self.atom.bus.memory.disk_interface_fitted,
                    Machine::GdosAtom => true,
                    Machine::SdCard | Machine::BbcBasic => false,
                };
                let why_not = match machine {
                    Machine::SdCard => Some(
                        "The SD card machine has no floppy drives. Disk images on the card are \
                         used with SDDOS (type SDDOS, then *DIN). Use Hardware > Machine to \
                         switch to a floppy disk Atom.",
                    ),
                    Machine::BbcBasic => Some(
                        "The BBC BASIC machine has no floppy drives. Use Hardware > Machine to \
                         switch to a floppy disk Atom.",
                    ),
                    _ if !drives_work => Some("No disk ROM is loaded (roms/dosrom.rom)."),
                    _ => None,
                };
                if let Some(reason) = why_not {
                    ui.add(egui::Label::new(egui::RichText::new(reason).weak()).wrap());
                }
                ui.add_enabled_ui(drives_work, |ui| {
                    for drive in 0..2 {
                        let disk = &self.atom.bus.fdc.drives[drive];
                        let (present, modified) =
                            (disk.is_some(), disk.as_ref().is_some_and(|d| d.modified));
                        let title = match (present, modified) {
                            (false, _) => format!("Drive {drive}: empty"),
                            (true, false) => format!("Drive {drive}"),
                            (true, true) => format!("Drive {drive} (unsaved changes)"),
                        };
                        let menu = ui.menu_button(title, |ui| {
                            if ui.button("Insert disk image...").clicked() {
                                self.insert_disk(drive);
                                ui.close();
                            }
                            if ui.button("New blank disk").clicked() {
                                self.new_blank_disk(drive);
                                ui.close();
                            }
                            if let Some(disk) = self.atom.bus.fdc.drives[drive].as_mut() {
                                ui.checkbox(&mut disk.write_protected, "Write protected");
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
                        if let Some(reason) = why_not {
                            menu.response.on_disabled_hover_text(reason);
                        }
                    }
                });
            });
            ui.menu_button("Sound", |ui| {
                if self.audio.is_none() {
                    ui.label("No audio device available");
                }
                ui.checkbox(&mut self.muted, "Mute");
                ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).text("volume"));
                ui.label("Sound plays at normal speed or slower.");
            });
            ui.menu_button("Hardware", |ui| {
                ui.set_max_width(330.0);
                let machine = self.atom.machine();

                // The kind of Atom comes first: everything below depends on it.
                ui.label(egui::RichText::new("Machine").strong());
                for choice in Machine::ALL {
                    let response = ui
                        .radio(machine == choice, choice.label())
                        .on_hover_text(choice.description());
                    if response.clicked() && machine != choice {
                        self.switch_machine(choice);
                        ui.close();
                    }
                }
                ui.add(egui::Label::new(egui::RichText::new(machine.description()).weak()).wrap());
                ui.label(egui::RichText::new("Changing the machine restarts the Atom.").weak());
                ui.separator();

                // Then the settings that belong to this machine.
                ui.label(
                    egui::RichText::new(format!("Settings for: {}", machine.label())).strong(),
                );
                match machine {
                    Machine::FloppyAtom | Machine::GdosAtom => {
                        ui.menu_button("Main RAM", |ui| {
                            let current = self.atom.bus.memory.ram_preset();
                            for preset in RamPreset::ALL {
                                let choice =
                                    egui::RadioButton::new(current == preset, preset.label());
                                if ui.add(choice).clicked() {
                                    self.typist.cancel();
                                    self.restart_audio();
                                    self.notice = self.atom.set_ram_preset(preset).err();
                                    ui.close();
                                }
                            }
                            ui.separator();
                            ui.label("Changing it restarts the Atom.");
                        });
                    }
                    Machine::SdCard | Machine::BbcBasic => {
                        ui.label("Main RAM: all 23K, from the RAM/ROM board.");
                        ui.label(
                            egui::RichText::new("(Choose a floppy disk machine to change it.)")
                                .weak(),
                        );
                    }
                }
                if machine == Machine::GdosAtom {
                    ui.menu_button("GDOS ROM bank", |ui| {
                        let current = self.atom.bus.memory.gdos_bank();
                        for bank in 0..GDOS_BANKS {
                            let choice =
                                egui::RadioButton::new(current == bank, format!("Bank {bank:02}"));
                            if ui.add(choice).clicked() {
                                self.typist.cancel();
                                self.restart_audio();
                                self.atom.bus.memory.set_gdos_bank(bank);
                                self.atom.reset();
                                ui.close();
                            }
                        }
                    });
                }
                if machine == Machine::SdCard {
                    if let Some(folder) = self.atom.bus.atommc.card_folder() {
                        // Two labels, the second cut short with the whole path
                        // on hover: one long wrapping label in a menu gets its
                        // first line stretched across the menu's width.
                        ui.label("Card folder:");
                        let path = folder.display().to_string();
                        ui.add(egui::Label::new(egui::RichText::new(&path).weak()).truncate())
                            .on_hover_text(path);
                    }
                    if ui.button("Choose another card folder...").clicked() {
                        self.choose_sd_card();
                        ui.close();
                    }
                }
                ui.separator();

                // Settings every machine has.
                ui.label(egui::RichText::new("All machines").strong());
                ui.menu_button("Video RAM", |ui| {
                    let current = self.atom.bus.memory.video_ram_kb();
                    for kb in 1..=MAX_VIDEO_RAM_KB {
                        let choice = egui::RadioButton::new(current == kb, format!("{kb}K"));
                        if ui.add(choice).clicked() {
                            self.atom.bus.memory.set_video_ram_kb(kb);
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.label("Graphics modes need more: 6K for the biggest.");
                });
                ui.checkbox(&mut self.colour_board, "Colour board");
                self.atom.bus.framebuffer.monochrome = !self.colour_board;
                ui.checkbox(&mut self.atom.bus.framebuffer.show_border, "Screen border")
                    .on_hover_text(
                        "The border the 6847 draws round the picture: black with text, green or buff with graphics.",
                    );
                ui.checkbox(&mut self.atom.bus.snow, "Video snow (experimental)")
                    .on_hover_text(
                        "Flicker from the CPU and the 6847 sharing video RAM. An approximation.",
                    );
                ui.checkbox(&mut self.atom.bus.memory.sid_fitted, "SID sound card");
                ui.add_enabled_ui(self.atom.bus.memory.sid_fitted, |ui| {
                    ui.menu_button("SID address", |ui| {
                        ui.label("Software is written for one or the other.");
                        for address in SidAddress::ALL {
                            let current = self.atom.bus.memory.sid_address;
                            let choice = egui::RadioButton::new(current == address, address.label());
                            if ui.add(choice).clicked() && current != address {
                                self.atom.bus.memory.sid_address = address;
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("SID model", |ui| {
                        let current = self.atom.bus.sid.model();
                        for model in SidModel::ALL {
                            let choice = egui::RadioButton::new(current == model, model.label());
                            if ui.add(choice).clicked() {
                                self.atom.bus.sid.set_model(model);
                                ui.close();
                            }
                        }
                    });
                });
                ui.menu_button("ROMs", |ui| {
                    ui.menu_button("Load a single ROM", |ui| {
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
                });
            });
            ui.menu_button("Input", |ui| {
                ui.checkbox(&mut self.joystick_keys, "Joystick on the keyboard");
                ui.label("Arrow keys steer, Space fires.");
                ui.separator();
                if ui.button("Keyboard mapping...").clicked() {
                    self.keyboard_window.open = true;
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.integer_scaling, "Integer scaling");
                let mut fullscreen = self.fullscreen;
                if ui.checkbox(&mut fullscreen, "Full screen (F11)").changed() {
                    let ctx = ui.ctx().clone();
                    self.set_fullscreen(&ctx, fullscreen);
                }
            });
            ui.menu_button("Debug", |ui| {
                let windows = &mut self.debug_windows;
                ui.checkbox(&mut windows.show_cpu, "CPU and controls");
                ui.checkbox(&mut windows.show_disassembly, "Disassembly");
                ui.checkbox(&mut windows.show_memory, "Memory");
                ui.checkbox(&mut windows.show_breakpoints, "Breakpoints");
                ui.checkbox(&mut windows.show_console, "Command box");
                ui.checkbox(&mut windows.show_history, "Instruction history");
                ui.checkbox(&mut windows.show_hardware, "Chip registers");
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Help (F1)").clicked() {
                    self.help.help_open = true;
                    ui.close();
                }
                if ui.button("About atom-rs").clicked() {
                    self.help.about_open = true;
                    ui.close();
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let regs = self.atom.cpu.regs.clone();
        ui.horizontal(|ui| {
            ui.label(if self.session.running {
                "Running"
            } else {
                "Paused"
            });
            if let Some(message) = &self.session.stop_message {
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
            ui.separator();
            let machine = self.atom.machine();
            match self.atom.bus.atommc.card_folder() {
                Some(folder) if machine == Machine::SdCard => {
                    let name = folder
                        .file_name()
                        .map_or_else(|| "?".into(), |n| n.to_string_lossy());
                    ui.label(format!("{}: {name}", machine.label()));
                }
                _ => {
                    ui.label(machine.label());
                }
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
        let (width, height) = self.atom.bus.framebuffer.shown_size();
        let (width, height) = (width as f32, height as f32);
        let mut scale = (available.x / width).min(available.y / height);
        if self.integer_scaling && scale >= 1.0 {
            scale = scale.floor();
        }
        let size = Vec2::new(width, height) * scale;
        ui.centered_and_justified(|ui| {
            ui.add(egui::Image::new((texture, size)));
        });
    }
}

impl eframe::App for AtomApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_keyboard(ctx);
        self.poll_archive_job();
        self.run_emulation();
        ctx.request_repaint(); // keep emulating even when nothing else changes
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| self.screen(ui));

        // F1 opens the help. (It is not an Atom key.)
        if ui.input(|i| i.key_pressed(Key::F1)) {
            self.help.help_open = true;
        }
        self.help.show(ui.ctx());
        self.tape_catalogue
            .show(ui.ctx(), self.atom.bus.tape.image());

        // The keyboard mapping window; save the map whenever it changes.
        if self.keyboard_window.show(ui.ctx(), &mut self.keymap)
            && let Err(message) = self.keymap.save()
        {
            self.notice = Some(format!("Cannot save the key map: {message}"));
        }

        self.save_settings_if_changed(ui.ctx());

        // Debugger windows are separate OS windows; they hand back commands.
        let commands = self
            .debug_windows
            .show(ui.ctx(), &mut self.atom, &mut self.session);
        for command in commands {
            self.execute(command);
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::cli::Options;
    use atom_rs::machine::memory::RamPreset;
    use atom_rs::machine::sid::SidModel;

    fn app() -> Option<AtomApp> {
        let mut app = AtomApp::new(PathBuf::from("roms"), Options::default());
        if app.rom_error.is_some() {
            return None;
        }
        // `new` restores whatever settings file is on this computer; start
        // every test from the plain floppy disk Atom instead.
        app.atom
            .set_machine(Machine::FloppyAtom, &app.rom_dir, None)
            .ok()?;
        app.notice = None;
        Some(app)
    }

    /// Saved settings really reach the machine, and reading them back gives
    /// the same settings (so nothing is lost or changed on the way).
    #[test]
    fn applying_settings_puts_them_into_effect() {
        let Some(mut app) = app() else { return };
        let wanted = Settings {
            machine: Machine::GdosAtom,
            ram_preset: RamPreset::With16KExpansion,
            video_ram_kb: 3,
            colour_board: false,
            show_border: false,
            snow: true,
            sid_fitted: false,
            sid_address: SidAddress::NineFc0,
            sid_model: SidModel::Mos8580,
            volume: 0.7,
            muted: true,
            integer_scaling: false,
            fast_tape: true,
            joystick_keys: true,
            ..Settings::default()
        };
        app.apply_settings(&wanted, true);
        assert_eq!(app.atom.machine(), Machine::GdosAtom);
        assert!(app.atom.bus.snow && app.atom.bus.framebuffer.monochrome);
        assert_eq!(app.settings(), wanted);
    }

    #[test]
    fn the_sd_card_machine_without_its_folder_falls_back_with_a_message() {
        let Some(mut app) = app() else { return };
        let wanted = Settings {
            machine: Machine::SdCard,
            card_folder: None,
            ..Settings::default()
        };
        app.apply_settings(&wanted, true);
        assert_eq!(app.atom.machine(), Machine::FloppyAtom);
        assert!(app.notice.is_some_and(|n| n.contains("saved machine")));
    }

    /// A command line that names the machine (here: a state file) wins.
    #[test]
    fn the_saved_machine_is_not_restored_when_told_not_to() {
        let Some(mut app) = app() else { return };
        let saved = Settings {
            machine: Machine::BbcBasic,
            ..Settings::default()
        };
        app.apply_settings(&saved, false);
        assert_eq!(app.atom.machine(), Machine::FloppyAtom);
    }
}

#[cfg(test)]
mod archive_job_tests {
    use super::*;
    use crate::cli::Options;
    use std::time::{Duration, Instant};

    fn app() -> Option<AtomApp> {
        let mut app = AtomApp::new(PathBuf::from("roms"), Options::default());
        if app.rom_error.is_some() || !Path::new("roms/ramrom.rom").exists() {
            return None;
        }
        app.atom
            .set_machine(Machine::FloppyAtom, &app.rom_dir, None)
            .ok()?;
        app.card_folder = None;
        Some(app)
    }

    fn poll_until_finished(app: &mut AtomApp) {
        let start = Instant::now();
        while app.archive_job.is_some() && start.elapsed() < Duration::from_secs(5) {
            app.poll_archive_job();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.archive_job.is_none(), "the job never finished");
    }

    #[test]
    fn when_the_archive_arrives_the_sd_card_machine_starts_with_it() {
        let Some(mut app) = app() else { return };
        let folder = std::env::temp_dir().join("atom-rs-test-archive");
        std::fs::create_dir_all(&folder).unwrap();
        let sent = folder.clone();
        app.archive_job = Some(ArchiveJob::spawn(move |say| {
            say("Downloading archive...");
            Ok(sent)
        }));
        poll_until_finished(&mut app);
        assert_eq!(app.atom.machine(), Machine::SdCard);
        assert_eq!(app.card_folder.as_deref(), Some(folder.as_path()));
        assert!(app.notice.is_some_and(|n| n.starts_with("SD card:")));
    }

    #[test]
    fn a_failed_download_says_why_and_leaves_the_machine_alone() {
        let Some(mut app) = app() else { return };
        app.archive_job = Some(ArchiveJob::spawn(|_| Err("no network".to_string())));
        poll_until_finished(&mut app);
        assert_eq!(app.atom.machine(), Machine::FloppyAtom);
        let notice = app.notice.unwrap();
        assert!(
            notice.contains("no network") && notice.contains("--sdcard"),
            "{notice}"
        );
    }

    #[test]
    fn choosing_another_machine_cancels_the_wait() {
        let Some(mut app) = app() else { return };
        app.archive_job = Some(ArchiveJob::spawn(|_| {
            std::thread::sleep(Duration::from_millis(300));
            Ok(PathBuf::from("/nowhere"))
        }));
        app.switch_machine(Machine::GdosAtom);
        assert!(app.archive_job.is_none());
        std::thread::sleep(Duration::from_millis(400));
        app.poll_archive_job();
        assert_eq!(
            app.atom.machine(),
            Machine::GdosAtom,
            "the late archive must not switch it"
        );
    }
}

#[cfg(test)]
mod loading_tests {
    use super::*;
    use crate::cli::Options;
    use atom_rs::machine::tape::TapeChunk;

    fn app() -> Option<AtomApp> {
        let mut app = AtomApp::new(PathBuf::from("roms"), Options::default());
        if app.rom_error.is_some() {
            return None;
        }
        app.atom
            .set_machine(Machine::FloppyAtom, &app.rom_dir, None)
            .ok()?;
        app.notice = None;
        Some(app)
    }

    /// A scratch folder that is removed afterwards.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!(
                "atom-rs-app-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn notice(app: &AtomApp) -> &str {
        app.notice.as_deref().unwrap_or("")
    }

    #[test]
    fn a_disk_file_goes_into_the_drive_and_its_path_is_remembered() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("disk");
        let ssd = folder.0.join("one.ssd");
        std::fs::write(&ssd, DiskImage::blank_80_track(1).to_bytes()).unwrap();
        app.insert_disk_file(1, &ssd);
        assert!(app.atom.bus.fdc.drives[1].is_some());
        assert!(
            app.atom.bus.fdc.drives[0].is_none(),
            "the other drive is untouched"
        );
        assert_eq!(app.disk_paths[1].as_deref(), Some(ssd.as_path()));
        assert_eq!(app.atom.bus.fdc.drives[1].as_ref().unwrap().sides, 1);
        assert!(notice(&app).contains("one.ssd"), "{}", notice(&app));
    }

    #[test]
    fn a_dsd_file_is_read_as_double_sided() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("dsd");
        let dsd = folder.0.join("two.DSD");
        std::fs::write(&dsd, DiskImage::blank_80_track(2).to_bytes()).unwrap();
        app.insert_disk_file(0, &dsd);
        assert_eq!(app.atom.bus.fdc.drives[0].as_ref().unwrap().sides, 2);
    }

    #[test]
    fn a_missing_or_bad_disk_file_says_so_and_changes_nothing() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("baddisk");
        app.insert_disk_file(0, &folder.0.join("missing.ssd"));
        assert!(notice(&app).starts_with("Cannot load"), "{}", notice(&app));
        let junk = folder.0.join("junk.ssd");
        std::fs::write(&junk, b"too short to be a disk").unwrap();
        app.insert_disk_file(0, &junk);
        assert!(notice(&app).starts_with("Cannot load"), "{}", notice(&app));
        assert!(app.atom.bus.fdc.drives[0].is_none());
        assert!(app.disk_paths[0].is_none());
    }

    #[test]
    fn saving_a_disk_writes_it_back_to_its_file_and_clears_the_modified_flag() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("savedisk");
        let path = folder.0.join("d.ssd");
        std::fs::write(&path, DiskImage::blank_80_track(1).to_bytes()).unwrap();
        app.insert_disk_file(0, &path);
        let disk = app.atom.bus.fdc.drives[0].as_mut().unwrap();
        disk.write_sector(0, 0, 0, &[0x42; 256]);
        assert!(disk.modified);
        app.save_disk(0, false);
        assert!(notice(&app).starts_with("Saved"), "{}", notice(&app));
        assert!(!app.atom.bus.fdc.drives[0].as_ref().unwrap().modified);
        assert_eq!(std::fs::read(&path).unwrap()[0], 0x42);
    }

    #[test]
    fn a_blank_disk_has_no_file_and_ejecting_removes_disk_and_path() {
        let Some(mut app) = app() else { return };
        app.disk_paths[0] = Some(PathBuf::from("old.ssd"));
        app.new_blank_disk(0);
        assert!(app.atom.bus.fdc.drives[0].is_some());
        assert!(app.disk_paths[0].is_none());
        assert!(notice(&app).contains("Blank"));
        app.eject_disk(0);
        assert!(app.atom.bus.fdc.drives[0].is_none());
        app.save_disk(0, false); // nothing in the drive: nothing happens, no dialog
    }

    #[test]
    fn a_tape_file_is_inserted_and_a_bad_one_reported() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("tape");
        let good = folder.0.join("t.uef");
        let image = atom_rs::machine::tape::TapeImage {
            chunks: vec![TapeChunk::Carrier { cycles: 10 }, TapeChunk::Byte(1)],
        };
        std::fs::write(&good, image.to_uef()).unwrap();
        app.insert_tape_file(&good);
        assert!(app.atom.bus.tape.has_tape());
        assert!(notice(&app).starts_with("Inserted"), "{}", notice(&app));

        app.atom.bus.tape.eject();
        let bad = folder.0.join("bad.uef");
        std::fs::write(&bad, b"not a tape").unwrap();
        app.insert_tape_file(&bad);
        assert!(!app.atom.bus.tape.has_tape());
        assert!(notice(&app).starts_with("Cannot load"), "{}", notice(&app));
        app.insert_tape_file(&folder.0.join("missing.uef"));
        assert!(notice(&app).starts_with("Cannot load"), "{}", notice(&app));
    }

    #[test]
    fn recording_starts_on_one_call_and_puts_the_tape_in_the_player_on_the_next() {
        let Some(mut app) = app() else { return };
        app.toggle_recording();
        assert!(app.atom.bus.tape.recording);
        for n in 0..9600 {
            app.atom.bus.tape.record(n % 2 == 0);
        }
        app.toggle_recording();
        assert!(!app.atom.bus.tape.recording);
        assert!(app.atom.bus.tape.has_tape());
        assert!(
            notice(&app).starts_with("Recorded 2.0s"),
            "{}",
            notice(&app)
        );
    }

    #[test]
    fn a_missing_state_file_is_reported_and_a_bad_one_leaves_the_machine_alone() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("state");
        app.load_state_path(&folder.0.join("nothing.state"));
        assert!(notice(&app).starts_with("Cannot read"), "{}", notice(&app));
        let junk = folder.0.join("junk.state");
        std::fs::write(&junk, b"junk").unwrap();
        app.atom.cpu.regs.a = 0x42;
        app.load_state_path(&junk);
        assert_eq!(app.atom.cpu.regs.a, 0x42);
        assert!(!notice(&app).starts_with("Restored"), "{}", notice(&app));
    }

    #[test]
    fn a_saved_state_comes_back_through_quick_save_and_quick_load() {
        let Some(mut app) = app() else { return };
        app.quick_load();
        let nothing_yet = notice(&app).to_string();
        app.atom.cpu.regs.a = 0x11;
        app.quick_save();
        app.atom.cpu.regs.a = 0x22;
        app.quick_load();
        assert_eq!(app.atom.cpu.regs.a, 0x11);
        assert_ne!(notice(&app), nothing_yet);
    }

    #[test]
    fn start_options_switch_things_on_and_load_the_files_named() {
        let Some(mut app) = app() else { return };
        let folder = Scratch::new("start");
        let ssd = folder.0.join("a.ssd");
        std::fs::write(&ssd, DiskImage::blank_80_track(1).to_bytes()).unwrap();
        let uef = folder.0.join("a.uef");
        std::fs::write(&uef, atom_rs::machine::tape::TapeImage::default().to_uef()).unwrap();
        let options = Options {
            joystick: true,
            fast_tape: true,
            disks: [Some(ssd), None],
            tape: Some(uef),
            run: Some("PRINT 1\n".to_string()),
            ..Options::default()
        };
        app.apply_start_options(&options);
        assert!(app.joystick_keys);
        assert!(app.fast_tape);
        assert!(app.autoplay_tape);
        assert!(app.atom.bus.fdc.drives[0].is_some());
        assert!(app.atom.bus.tape.has_tape());
    }

    #[test]
    fn start_options_never_switch_a_saved_setting_off() {
        let Some(mut app) = app() else { return };
        app.joystick_keys = true;
        app.fast_tape = true;
        app.apply_start_options(&Options::default());
        assert!(app.joystick_keys && app.fast_tape);
        assert!(!app.autoplay_tape);
    }

    #[test]
    fn nothing_is_applied_when_the_roms_could_not_be_loaded() {
        let Some(mut app) = app() else { return };
        app.rom_error = Some("no roms".into());
        app.apply_start_options(&Options {
            joystick: true,
            ..Options::default()
        });
        assert!(!app.joystick_keys);
    }

    #[test]
    fn the_effective_speed_follows_the_tape_when_fast_tape_is_on() {
        let Some(mut app) = app() else { return };
        app.speed = 1.0;
        app.fast_tape = true;
        assert_eq!(app.effective_speed(), 1.0);
        app.atom.bus.tape.playing = true;
        assert_eq!(app.effective_speed(), logic::MAX_SPEED);
    }

    // ---- Drawing -----------------------------------------------------------------------------

    /// Draw the three main panels once (the menu, status bar and screen) the
    /// way `ui` does, without touching the settings file.
    fn draw(app: &mut AtomApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::Panel::top("menu").show(ui, |ui| app.menu_bar(ui));
            egui::Panel::bottom("status").show(ui, |ui| app.status_bar(ui));
            egui::CentralPanel::default().show(ui, |ui| app.screen(ui));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_main_window_draws_in_every_state_it_can_be_in() {
        let Some(mut app) = app() else { return };
        let ctx = egui::Context::default();
        draw(&mut app, &ctx, vec![]);

        app.notice = Some("A notice for the status bar".to_string());
        app.session.running = false;
        draw(&mut app, &ctx, vec![]);

        app.session.running = true;
        app.atom.bus.fdc.drives[0] = Some(DiskImage::blank_80_track(2));
        app.atom.bus.fdc.drives[1] = Some(DiskImage::blank_80_track(1));
        app.disk_paths[0] = Some(PathBuf::from("a.dsd"));
        app.atom.bus.tape.insert(atom_rs::machine::tape::TapeImage {
            chunks: vec![atom_rs::machine::tape::TapeChunk::Byte(1)],
        });
        app.atom.bus.tape.playing = true;
        app.fast_tape = true;
        app.muted = true;
        app.integer_scaling = true;
        app.autoplay_tape = true;
        app.atom.bus.tape.recording = true;
        draw(&mut app, &ctx, vec![]);
        app.fullscreen = true;
        app.joystick_keys = true;
        app.colour_board = false;
        draw(&mut app, &ctx, vec![]);
    }

    #[test]
    fn the_window_shows_the_problem_instead_of_the_screen_if_the_roms_are_missing() {
        let Some(mut app) = app() else { return };
        app.rom_error = Some("Cannot find os.rom".to_string());
        draw(&mut app, &egui::Context::default(), vec![]);
    }

    #[test]
    fn every_machine_can_be_drawn_after_switching_to_it() {
        let Some(mut app) = app() else { return };
        let ctx = egui::Context::default();
        for machine in [Machine::FloppyAtom, Machine::GdosAtom, Machine::BbcBasic] {
            app.switch_machine(machine);
            draw(&mut app, &ctx, vec![]);
        }
    }

    #[test]
    fn every_debugger_window_draws_and_a_paused_machine_can_be_stepped() {
        let Some(mut app) = app() else { return };
        let ctx = egui::Context::default();
        app.debug_windows.show_cpu = true;
        app.debug_windows.show_disassembly = true;
        app.debug_windows.show_memory = true;
        app.debug_windows.show_breakpoints = true;
        app.debug_windows.show_hardware = true;
        app.debug_windows.show_console = true;
        app.debug_windows.show_history = true;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let commands = app
                .debug_windows
                .show(ui.ctx(), &mut app.atom, &mut app.session);
            for command in commands {
                app.execute(command);
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn running_the_emulation_advances_the_machine_in_real_time_and_stops_when_paused() {
        let Some(mut app) = app() else { return };
        app.last_update = Instant::now() - Duration::from_millis(50);
        let before = app.atom.bus.cycles;
        app.run_emulation();
        assert!(
            app.atom.bus.cycles > before,
            "50 ms of real time runs some cycles"
        );

        app.session.running = false;
        app.last_update = Instant::now() - Duration::from_millis(50);
        let paused_at = app.atom.bus.cycles;
        app.run_emulation();
        assert_eq!(
            app.atom.bus.cycles, paused_at,
            "a paused machine does not run"
        );
    }

    #[test]
    fn host_keys_held_down_reach_the_atoms_keyboard() {
        let Some(mut app) = app() else { return };
        let ctx = egui::Context::default();
        let key = |key: Key, pressed: bool| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let input = egui::RawInput {
            events: vec![key(Key::A, true)],
            ..Default::default()
        };
        ctx.run_ui(input, |_| {}).textures_delta.clear();
        app.update_keyboard(&ctx);
        let (row, column) = atom_rs::machine::keyboard::find_key("A").unwrap();
        assert_eq!(
            app.atom.bus.ppi.keyboard.read_port_b(row as u8) & (1 << column),
            0,
            "A is held down on the Atom"
        );
        let input = egui::RawInput {
            events: vec![key(Key::A, false)],
            ..Default::default()
        };
        ctx.run_ui(input, |_| {}).textures_delta.clear();
        app.update_keyboard(&ctx);
        assert_ne!(
            app.atom.bus.ppi.keyboard.read_port_b(row as u8) & (1 << column),
            0
        );
    }
}
