//! Acorn Atom emulator.
//!
//! The crate is split into modules with a strict dependency direction:
//! `ui` -> `machine` -> `cpu`. The `cpu` module must never import from
//! `machine` or `ui`, so it can be tested on its own.

pub mod cpu;
pub mod machine;
