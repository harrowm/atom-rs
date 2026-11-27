//! The Acorn Atom itself: memory map, peripherals and the machine that ties
//! them to the CPU.
//!
//! Dependencies point one way: `machine` uses `cpu`, never the reverse.

pub mod atm;
pub mod atom;
pub mod atommc;
pub mod autoplay;
pub mod csw;
pub mod debug;
pub mod debug_console;
pub mod debug_session;
pub mod disk;
pub mod fdc1770;
pub mod fdc8271;
pub mod font;
pub mod joystick;
pub mod keyboard;
pub mod memory;
pub mod ppi8255;
pub mod ramrom;
pub mod sddos;
pub mod sid;
pub mod snapshot;
pub mod sound;
pub mod tape;
pub mod typist;
pub mod vdg6847;
pub mod via6522;
pub mod video;

pub use atom::{Atom, AtomBus};
