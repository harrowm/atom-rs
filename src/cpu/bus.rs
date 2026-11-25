//! The bus: the only way the CPU talks to the rest of the machine.
//!
//! On a real 6502 every clock cycle performs exactly one bus access, either a
//! read or a write. The CPU core mirrors that: each call to [`Bus::read`] or
//! [`Bus::write`] *is* one clock cycle. A machine implementing this trait can
//! therefore advance its other chips (timers, video, sound) inside those
//! methods and stay in lock-step with the processor.

/// A 16-bit address bus with 8-bit data.
pub trait Bus {
    /// One read cycle.
    fn read(&mut self, addr: u16) -> u8;

    /// One write cycle.
    fn write(&mut self, addr: u16, value: u8);

    /// Is the (level-triggered, active) IRQ line being held asserted?
    fn irq_asserted(&mut self) -> bool {
        false
    }

    /// Has a falling edge occurred on the NMI line since the last call?
    /// The CPU calls this once per poll, so implementations must clear their
    /// "edge seen" flag when it is read.
    fn take_nmi_edge(&mut self) -> bool {
        false
    }
}

/// A plain 64 KiB of RAM with no devices. Handy for tests.
pub struct RamBus {
    pub memory: Box<[u8; 0x10000]>,
}

impl RamBus {
    pub fn new() -> Self {
        RamBus {
            memory: Box::new([0; 0x10000]),
        }
    }
}

impl Default for RamBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for RamBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.memory[addr as usize]
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.memory[addr as usize] = value;
    }
}
