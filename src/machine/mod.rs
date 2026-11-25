//! The Acorn Atom itself: memory map, peripherals and the machine that ties
//! them to the CPU.
//!
//! Dependencies point one way: `machine` uses `cpu`, never the reverse.

pub mod atm;
pub mod atom;
pub mod debug;
pub mod disk;
pub mod fdc8271;
pub mod font;
pub mod keyboard;
pub mod memory;
pub mod ppi8255;
pub mod snapshot;
pub mod sound;
pub mod tape;
pub mod vdg6847;
pub mod via6522;
pub mod video;

pub use atom::{Atom, AtomBus};
