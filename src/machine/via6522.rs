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

    // ---- Registers ------------------------------------------------------------------------

    const ORB: u16 = 0x0;
    const ORA: u16 = 0x1;
    const DDRB: u16 = 0x2;
    const DDRA: u16 = 0x3;
    const T1C_L: u16 = 0x4;
    const T1C_H: u16 = 0x5;
    const T1L_L: u16 = 0x6;
    const T1L_H: u16 = 0x7;
    const T2C_L: u16 = 0x8;
    const T2C_H: u16 = 0x9;
    const SR: u16 = 0xA;
    const ACR: u16 = 0xB;
    const PCR: u16 = 0xC;
    const IFR: u16 = 0xD;
    const IER: u16 = 0xE;
    const ORA_NO_HANDSHAKE: u16 = 0xF;

    #[test]
    fn the_registers_that_just_hold_a_byte_read_back_what_was_written() {
        let mut via = Via::new();
        for (register, value) in [
            (DDRA, 0xA5),
            (DDRB, 0x5A),
            (SR, 0x3C),
            (ACR, 0x41),
            (PCR, 0xCE),
        ] {
            via.write(register, value);
            assert_eq!(via.read(register), value, "register {register:X}");
        }
    }

    #[test]
    fn the_registers_repeat_every_sixteen_addresses() {
        let mut via = Via::new();
        via.write(0xB800 + DDRA, 0x77);
        assert_eq!(
            via.read(0xBFF0 + DDRA),
            0x77,
            "only the low four address bits choose the register"
        );
    }

    #[test]
    fn the_power_on_state_has_everything_off() {
        let mut via = Via::new();
        let state = via.snapshot();
        assert_eq!(
            (state.direction_a, state.direction_b),
            (0, 0),
            "every pin an input"
        );
        assert_eq!((state.output_a, state.output_b), (0, 0));
        assert_eq!((state.aux_control, state.peripheral_control), (0, 0));
        assert_eq!((state.flags, state.enables), (0, 0));
        assert!(!via.irq());
        assert_eq!(via.read(IER), 0x80, "IER reads with bit 7 set");
        assert_eq!(via.read(IFR), 0);
        let again = Via::default().snapshot();
        assert_eq!(
            (again.flags, again.timer1, again.timer2),
            (state.flags, state.timer1, state.timer2)
        );
    }

    #[test]
    fn port_b_mixes_its_output_bits_and_its_input_pins_by_the_direction_register() {
        let mut via = Via::new();
        via.input_b = 0b0000_1111;
        via.write(DDRB, 0b1111_0000);
        via.write(ORB, 0b1010_1010);
        assert_eq!(
            via.read(ORB),
            0b1010_1111,
            "high nibble from the register, low from the pins"
        );
        via.write(DDRB, 0);
        assert_eq!(via.read(ORB), 0b0000_1111, "all inputs: only the pins");
    }

    #[test]
    fn port_a_without_handshake_is_port_a_but_leaves_the_handshake_flags_alone() {
        let mut via = Via::new();
        via.write(DDRA, 0xFF);
        via.write(ORA_NO_HANDSHAKE, 0x42);
        assert_eq!(via.read(ORA_NO_HANDSHAKE), 0x42);
        assert_eq!(via.read(ORA), 0x42, "the same register");
        via.flags = irq::CA1 | irq::CA2;
        via.read(ORA_NO_HANDSHAKE);
        via.write(ORA_NO_HANDSHAKE, 1);
        assert_eq!(
            via.flags,
            irq::CA1 | irq::CA2,
            "no handshake: the flags stay"
        );
        via.read(ORA);
        assert_eq!(via.flags, 0, "the ordinary port A access clears them");
    }

    // ---- Interrupt flags and enables -------------------------------------------------------

    #[test]
    fn the_enable_register_sets_and_clears_by_bit_seven() {
        let mut via = Via::new();
        via.write(IER, 0x80 | irq::TIMER1 | irq::CB1);
        assert_eq!(
            via.read(IER),
            0x80 | irq::TIMER1 | irq::CB1,
            "bit 7 always reads 1"
        );
        via.write(IER, irq::TIMER1); // bit 7 clear: the 1 bits are cleared
        assert_eq!(via.read(IER), 0x80 | irq::CB1);
        via.write(IER, 0xFF);
        assert_eq!(via.read(IER), 0xFF, "all seven");
        via.write(IER, 0x7F);
        assert_eq!(via.read(IER), 0x80, "none");
    }

    #[test]
    fn a_flag_is_cleared_by_writing_a_one_to_it_and_not_by_writing_a_zero() {
        let mut via = Via::new();
        via.flags = irq::TIMER1 | irq::TIMER2 | irq::CB1;
        via.write(IFR, irq::TIMER2);
        assert_eq!(
            via.flags,
            irq::TIMER1 | irq::CB1,
            "only the written bit went"
        );
        via.write(IFR, 0x80); // bit 7 is not a flag
        via.write(IFR, 0x00);
        assert_eq!(via.flags, irq::TIMER1 | irq::CB1);
        via.write(IFR, 0x7F);
        assert_eq!(via.flags, 0);
    }

    #[test]
    fn the_irq_output_needs_a_flag_that_is_enabled_and_bit_seven_of_the_flags_shows_it() {
        let mut via = Via::new();
        via.flags = irq::TIMER1;
        assert!(!via.irq(), "flagged but not enabled");
        assert_eq!(via.read(IFR), irq::TIMER1, "bit 7 clear");
        via.write(IER, 0x80 | irq::TIMER2);
        assert!(!via.irq(), "another one enabled");
        via.write(IER, 0x80 | irq::TIMER1);
        assert!(via.irq());
        assert_eq!(
            via.read(IFR),
            0x80 | irq::TIMER1,
            "bit 7: an enabled interrupt is active"
        );
        via.write(IER, irq::TIMER1);
        assert!(!via.irq());
    }

    #[test]
    fn reading_and_writing_the_ports_clears_their_handshake_flags() {
        let all = irq::CA1 | irq::CA2 | irq::CB1 | irq::CB2 | irq::TIMER1;
        for (access, cleared) in [
            ("read ORA", irq::CA1 | irq::CA2),
            ("write ORA", irq::CA1 | irq::CA2),
            ("read ORB", irq::CB1 | irq::CB2),
            ("write ORB", irq::CB1 | irq::CB2),
        ] {
            let mut via = Via::new();
            via.flags = all;
            match access {
                "read ORA" => drop(via.read(ORA)),
                "write ORA" => via.write(ORA, 1),
                "read ORB" => drop(via.read(ORB)),
                _ => via.write(ORB, 1),
            }
            assert_eq!(via.flags, all & !cleared, "{access}");
        }
    }

    #[test]
    fn the_timer_flags_are_cleared_by_reading_the_low_counter_or_writing_the_high_one() {
        for (what, clears_t1, clears_t2) in [
            ("read T1C-L", true, false),
            ("write T1C-H", true, false),
            ("write T1L-H", true, false),
            ("read T2C-L", false, true),
            ("write T2C-H", false, true),
            ("read T1C-H", false, false),
            ("read T1L-L", false, false),
            ("write T1L-L", false, false),
        ] {
            let mut via = Via::new();
            via.flags = irq::TIMER1 | irq::TIMER2;
            match what {
                "read T1C-L" => drop(via.read(T1C_L)),
                "write T1C-H" => via.write(T1C_H, 1),
                "write T1L-H" => via.write(T1L_H, 1),
                "read T2C-L" => drop(via.read(T2C_L)),
                "write T2C-H" => via.write(T2C_H, 1),
                "read T1C-H" => drop(via.read(T1C_H)),
                "read T1L-L" => drop(via.read(T1L_L)),
                _ => via.write(T1L_L, 1),
            }
            assert_eq!(via.flags & irq::TIMER1 == 0, clears_t1, "{what}: T1 flag");
            assert_eq!(via.flags & irq::TIMER2 == 0, clears_t2, "{what}: T2 flag");
        }
    }

    // ---- Timer 1 ---------------------------------------------------------------------------

    #[test]
    fn the_timer_1_latch_and_counter_are_separate_until_the_high_byte_is_written() {
        let mut via = Via::new();
        via.write(T1C_L, 0x34); // goes to the latch only
        via.write(T1L_H, 0x12); // latch high, without starting the timer
        assert_eq!((via.read(T1L_L), via.read(T1L_H)), (0x34, 0x12));
        assert_eq!(
            via.snapshot().timer1,
            0xFFFF,
            "the counter has not been loaded"
        );
        via.write(T1C_H, 0x12); // loads the counter from the latch and starts it
        assert_eq!(via.snapshot().timer1, 0x1234);
        run(&mut via, 4);
        assert_eq!(
            (via.read(T1C_H), via.read(T1C_L)),
            (0x12, 0x30),
            "it counts down"
        );
    }

    #[test]
    fn in_one_shot_mode_the_counter_carries_on_down_after_the_interrupt_without_another() {
        let mut via = Via::new();
        via.write(T1C_L, 3);
        via.write(T1C_H, 0);
        run(&mut via, 4);
        assert_eq!(via.flags & irq::TIMER1, irq::TIMER1);
        via.read(T1C_L); // acknowledge
        run(&mut via, 70_000);
        assert_eq!(via.flags & irq::TIMER1, 0, "a one-shot interrupts once");
    }

    #[test]
    fn pb7_goes_low_when_a_one_shot_starts_and_high_when_it_times_out() {
        let mut via = Via::new();
        via.write(ACR, 0x80); // PB7 is the timer output, one-shot
        via.write(T1C_L, 5);
        via.write(T1C_H, 0);
        assert_eq!(via.read(ORB) & 0x80, 0, "low while the timer runs");
        run(&mut via, 5);
        assert_eq!(via.read(ORB) & 0x80, 0);
        run(&mut via, 1);
        assert_eq!(via.read(ORB) & 0x80, 0x80, "high again at the timeout");
    }

    #[test]
    fn pb7_toggles_at_every_timeout_of_the_free_running_timer() {
        let mut via = Via::new();
        via.write(ACR, 0xC0); // PB7 output, free-running
        via.write(T1C_L, 3);
        via.write(T1C_H, 0);
        assert_eq!(via.read(ORB) & 0x80, 0);
        run(&mut via, 4);
        assert_eq!(via.read(ORB) & 0x80, 0x80, "first timeout: high");
        run(&mut via, 5); // a period is N + 2
        assert_eq!(via.read(ORB) & 0x80, 0, "next: low");
        run(&mut via, 5);
        assert_eq!(via.read(ORB) & 0x80, 0x80);
    }

    #[test]
    fn pb7_shows_the_timer_only_when_the_aux_control_register_asks_for_it() {
        let mut via = Via::new();
        via.input_b = 0x80;
        via.write(DDRB, 0);
        assert_eq!(via.read(ORB) & 0x80, 0x80, "an input pin, as normal");
        via.write(ACR, 0x80);
        via.write(T1C_L, 9);
        via.write(T1C_H, 0);
        assert_eq!(via.read(ORB) & 0x80, 0, "now the timer's output");
        assert_eq!(via.read(ORB) & 0x7F, 0, "and the other pins are untouched");
    }

    #[test]
    fn a_free_running_timer_reloads_from_the_latch_each_time() {
        let mut via = Via::new();
        via.write(ACR, 0x40);
        via.write(T1C_L, 0x00);
        via.write(T1C_H, 0x01); // 256
        run(&mut via, 257);
        assert_eq!(via.flags & irq::TIMER1, irq::TIMER1);
        via.read(T1C_L);
        run(&mut via, 258);
        assert_eq!(
            via.flags & irq::TIMER1,
            irq::TIMER1,
            "and again, one period later"
        );
    }

    // ---- Timer 2 ---------------------------------------------------------------------------

    #[test]
    fn timer_2_counts_from_its_latch_low_byte_and_the_written_high_byte() {
        let mut via = Via::new();
        via.write(T2C_L, 0x10); // only the latch
        assert_eq!(via.snapshot().timer2, 0xFFFF);
        via.write(T2C_H, 0x00); // loads and starts
        assert_eq!(via.snapshot().timer2, 0x0010);
        run(&mut via, 16);
        assert_eq!(via.flags & irq::TIMER2, 0);
        run(&mut via, 1);
        assert_eq!(
            via.flags & irq::TIMER2,
            irq::TIMER2,
            "N + 1 cycles after the write"
        );
        via.read(T2C_L);
        run(&mut via, 100);
        assert_eq!(via.flags & irq::TIMER2, 0, "one-shot");
        assert_ne!(via.read(T2C_H), 0, "the counter kept going down past zero");
    }

    #[test]
    fn timer_2_pulse_counting_is_not_emulated_so_the_timer_just_stands_still() {
        // Documenting a known limitation: with ACR bit 5 set the real chip counts
        // pulses on PB6, which nothing drives here.
        let mut via = Via::new();
        via.write(ACR, 0x20);
        via.write(T2C_L, 5);
        via.write(T2C_H, 0);
        run(&mut via, 100);
        assert_eq!(via.snapshot().timer2, 5);
        assert_eq!(via.flags & irq::TIMER2, 0);
    }

    // ---- Found by mutation testing -----------------------------------------------------------------

    #[test]
    fn a_one_shot_counter_wraps_round_instead_of_reloading_from_the_latch() {
        let mut via = Via::new();
        via.write(T1C_L, 5);
        via.write(T1C_H, 0);
        for cycles_run in 1..=40 {
            via.tick();
            if cycles_run > 6 {
                // Past zero it has wrapped round to 0xFFxx; it never goes back to
                // the latch's 5 (which a free-running timer would).
                assert_eq!(via.read(T1C_H), 0xFF, "after {cycles_run} cycles");
            }
        }
    }

    #[test]
    fn starting_a_timer_does_not_touch_pb7_unless_pb7_output_is_on() {
        let mut via = Via::new();
        via.write(T1C_L, 5);
        via.write(T1C_H, 0); // starts the timer with PB7 output off
        via.write(ACR, 0x80); // now switch PB7 output on
        assert_eq!(via.read(ORB) & 0x80, 0x80, "PB7 was left high");
    }

    #[test]
    fn the_two_halves_of_the_timer_1_latch_and_timer_2_counter_go_to_the_right_bytes() {
        let mut via = Via::new();
        via.write(6, 0x34);
        via.write(7, 0x12);
        assert_eq!(via.t1_latch, 0x1234, "the high write keeps the low byte");
        via.write(0x8, 0x78);
        via.write(0x9, 0x56);
        assert_eq!(via.t2_counter, 0x5678);
    }

    #[test]
    fn port_a_reads_output_bits_from_the_register_and_input_bits_from_the_pins() {
        let mut via = Via::new();
        via.write(3, 0x0F); // low nibble output
        via.write(1, 0x05);
        via.input_a = 0xA0;
        assert_eq!(via.read(1), 0xA5);
        via.input_a = 0x00;
        assert_eq!(via.read(1), 0x05, "pins that read 0 stay 0");
        assert_eq!(
            via.read(0xF),
            0x05,
            "the no-handshake address reads the same"
        );
    }
}
