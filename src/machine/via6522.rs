//! The 6522 VIA at `B800`: two 8-bit ports and two 16-bit timers.
//!
//! The Atom uses it for the printer port, the user port and timing. Its
//! interrupt output is the Atom's IRQ line.
//!
//! Emulated: ports with data-direction registers, both timers (T1 one-shot
//! and free-running, T2 one-shot), the interrupt flag/enable registers and
//! the PB7 timer output. Not emulated: the control lines CA1/CA2/CB1/CB2 and
//! the shift register's shifting (the register just stores a byte).

use serde::{Deserialize, Serialize};

/// Interrupt flag bits, shared by the IFR and IER registers.
mod irq {
    pub const CA2: u8 = 0x01;
    pub const CA1: u8 = 0x02;
    pub const CB2: u8 = 0x08;
    pub const CB1: u8 = 0x10;
    pub const TIMER2: u8 = 0x20;
    pub const TIMER1: u8 = 0x40;
    /// In IFR: "any enabled interrupt active". In IER writes: set/clear mode.
    pub const ANY: u8 = 0x80;
}

/// Auxiliary control register bits.
mod acr {
    pub const T1_FREE_RUN: u8 = 0x40;
    pub const T1_PB7_OUTPUT: u8 = 0x80;
    pub const T2_PULSE_COUNT: u8 = 0x20;
}

/// A read-only copy of the VIA's registers, for the debugger.
#[derive(Clone, Copy, Debug)]
pub struct ViaSnapshot {
    pub output_a: u8,
    pub output_b: u8,
    pub direction_a: u8,
    pub direction_b: u8,
    pub timer1: u16,
    pub timer1_latch: u16,
    pub timer2: u16,
    pub aux_control: u8,
    pub peripheral_control: u8,
    pub flags: u8,
    pub enables: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Via {
    output_a: u8,
    output_b: u8,
    direction_a: u8,
    direction_b: u8,
    /// The levels outside the chip drive on pins configured as inputs.
    pub input_a: u8,
    pub input_b: u8,

    t1_counter: u16,
    t1_latch: u16,
    /// Will the next timeout set the T1 interrupt flag? (One-shot mode fires once.)
    t1_armed: bool,
    /// After a timeout the counter is reloaded from the latch one cycle later.
    t1_reload_pending: bool,
    pb7_level: bool,

    t2_counter: u16,
    t2_latch_low: u8,
    t2_armed: bool,

    shift_register: u8,
    aux_control: u8,
    peripheral_control: u8,
    flags: u8,
    enables: u8,
}

impl Via {
    pub fn new() -> Self {
        Via {
            output_a: 0,
            output_b: 0,
            direction_a: 0,
            direction_b: 0,
            input_a: 0,
            input_b: 0,
            t1_counter: 0xFFFF,
            t1_latch: 0xFFFF,
            t1_armed: false,
            t1_reload_pending: false,
            pb7_level: true,
            t2_counter: 0xFFFF,
            t2_latch_low: 0xFF,
            t2_armed: false,
            shift_register: 0,
            aux_control: 0,
            peripheral_control: 0,
            flags: 0,
            enables: 0,
        }
    }

    /// The registers as the debugger shows them (no side effects).
    pub fn snapshot(&self) -> ViaSnapshot {
        ViaSnapshot {
            output_a: self.output_a,
            output_b: self.output_b,
            direction_a: self.direction_a,
            direction_b: self.direction_b,
            timer1: self.t1_counter,
            timer1_latch: self.t1_latch,
            timer2: self.t2_counter,
            aux_control: self.aux_control,
            peripheral_control: self.peripheral_control,
            flags: self.flags,
            enables: self.enables,
        }
    }

    /// The IRQ output: asserted while any enabled interrupt flag is set.
    pub fn irq(&self) -> bool {
        self.flags & self.enables & 0x7F != 0
    }

    /// Advance one CPU cycle.
    pub fn tick(&mut self) {
        self.tick_timer1();
        self.tick_timer2();
    }

    fn tick_timer1(&mut self) {
        if self.t1_reload_pending {
            self.t1_counter = self.t1_latch;
            self.t1_reload_pending = false;
            return;
        }
        if self.t1_counter == 0 {
            if self.t1_armed {
                self.flags |= irq::TIMER1;
                if self.aux_control & acr::T1_FREE_RUN != 0 {
                    self.pb7_level = !self.pb7_level;
                } else {
                    self.t1_armed = false;
                    self.pb7_level = true;
                }
            }
            if self.aux_control & acr::T1_FREE_RUN != 0 {
                self.t1_reload_pending = true;
            }
        }
        self.t1_counter = self.t1_counter.wrapping_sub(1);
    }

    fn tick_timer2(&mut self) {
        if self.aux_control & acr::T2_PULSE_COUNT != 0 {
            return; // counts pulses on PB6, which is not emulated
        }
        if self.t2_counter == 0 && self.t2_armed {
            self.flags |= irq::TIMER2;
            self.t2_armed = false;
        }
        self.t2_counter = self.t2_counter.wrapping_sub(1);
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        match addr & 0xF {
            0x0 => {
                self.flags &= !(irq::CB1 | irq::CB2);
                self.read_port_b()
            }
            0x1 => {
                self.flags &= !(irq::CA1 | irq::CA2);
                self.read_port_a()
            }
            0x2 => self.direction_b,
            0x3 => self.direction_a,
            0x4 => {
                self.flags &= !irq::TIMER1;
                self.t1_counter as u8
            }
            0x5 => (self.t1_counter >> 8) as u8,
            0x6 => self.t1_latch as u8,
            0x7 => (self.t1_latch >> 8) as u8,
            0x8 => {
                self.flags &= !irq::TIMER2;
                self.t2_counter as u8
            }
            0x9 => (self.t2_counter >> 8) as u8,
            0xA => self.shift_register,
            0xB => self.aux_control,
            0xC => self.peripheral_control,
            0xD => self.read_flags(),
            0xE => self.enables | irq::ANY,
            _ => self.read_port_a(), // 0xF: port A without handshake
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 0xF {
            0x0 => {
                self.flags &= !(irq::CB1 | irq::CB2);
                self.output_b = value;
            }
            0x1 => {
                self.flags &= !(irq::CA1 | irq::CA2);
                self.output_a = value;
            }
            0x2 => self.direction_b = value,
            0x3 => self.direction_a = value,
            0x4 | 0x6 => self.t1_latch = (self.t1_latch & 0xFF00) | value as u16,
            0x5 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | ((value as u16) << 8);
                self.t1_counter = self.t1_latch;
                self.t1_armed = true;
                self.t1_reload_pending = false;
                self.flags &= !irq::TIMER1;
                if self.aux_control & acr::T1_PB7_OUTPUT != 0 {
                    self.pb7_level = false;
                }
            }
            0x7 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | ((value as u16) << 8);
                self.flags &= !irq::TIMER1;
            }
            0x8 => self.t2_latch_low = value,
            0x9 => {
                self.t2_counter = ((value as u16) << 8) | self.t2_latch_low as u16;
                self.t2_armed = true;
                self.flags &= !irq::TIMER2;
            }
            0xA => self.shift_register = value,
            0xB => self.aux_control = value,
            0xC => self.peripheral_control = value,
            0xD => self.flags &= !(value & 0x7F), // writing 1s clears flags
            0xE => {
                // Bit 7 chooses whether the 1-bits set or clear enables.
                if value & irq::ANY != 0 {
                    self.enables |= value & 0x7F;
                } else {
                    self.enables &= !(value & 0x7F);
                }
            }
            _ => self.output_a = value, // 0xF: port A without handshake
        }
    }

    /// Output bits come from the output register, input bits from the pins.
    fn read_port_a(&self) -> u8 {
        (self.output_a & self.direction_a) | (self.input_a & !self.direction_a)
    }

    fn read_port_b(&self) -> u8 {
        let mut value = (self.output_b & self.direction_b) | (self.input_b & !self.direction_b);
        if self.aux_control & acr::T1_PB7_OUTPUT != 0 {
            value = (value & 0x7F) | ((self.pb7_level as u8) << 7);
        }
        value
    }

    fn read_flags(&self) -> u8 {
        let any = if self.irq() { irq::ANY } else { 0 };
        (self.flags & 0x7F) | any
    }
}

impl Default for Via {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(via: &mut Via, cycles: u32) {
        for _ in 0..cycles {
            via.tick();
        }
    }

    #[test]
    fn output_pins_follow_direction_register() {
        let mut via = Via::new();
        via.input_a = 0b1111_0000;
        via.write(3, 0x0F); // low nibble output, high nibble input
        via.write(1, 0xFF);
        assert_eq!(via.read(1), 0b1111_1111);
        via.write(1, 0x00);
        assert_eq!(via.read(1), 0b1111_0000);
    }

    #[test]
    fn timer1_one_shot_interrupts_n_plus_1_cycles_after_the_write() {
        let mut via = Via::new();
        via.write(0xE, 0xC0); // enable the T1 interrupt
        via.write(4, 10);
        via.write(5, 0); // starts the timer: N = 10
        run(&mut via, 10);
        assert!(!via.irq());
        run(&mut via, 1);
        assert!(via.irq());
        assert_eq!(via.read(0xD), 0xC0); // any + T1
        via.read(4); // reading T1 low clears the flag
        assert!(!via.irq());
        run(&mut via, 100);
        assert!(!via.irq(), "one-shot mode fires only once");
    }

    #[test]
    fn timer1_free_run_repeats_every_n_plus_2_cycles() {
        let mut via = Via::new();
        via.write(0xB, 0x40); // free run
        via.write(0xE, 0xC0);
        via.write(4, 10);
        via.write(5, 0);
        run(&mut via, 11);
        assert!(via.irq());
        via.read(4);
        run(&mut via, 11);
        assert!(!via.irq());
        run(&mut via, 1);
        assert!(via.irq());
    }

    #[test]
    fn timer2_one_shot() {
        let mut via = Via::new();
        via.write(0xE, 0xA0); // enable the T2 interrupt
        via.write(8, 5);
        via.write(9, 0);
        run(&mut via, 5);
        assert!(!via.irq());
        run(&mut via, 1);
        assert!(via.irq());
        via.write(0xD, 0x20); // writing a 1 clears the flag
        assert!(!via.irq());
    }

    #[test]
    fn disabled_interrupts_do_not_assert_irq() {
        let mut via = Via::new();
        via.write(4, 2);
        via.write(5, 0);
        run(&mut via, 5);
        assert!(!via.irq());
        assert_eq!(via.read(0xD), 0x40, "flag set, but not 'any enabled'");
    }
}
