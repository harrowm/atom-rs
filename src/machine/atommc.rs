//! A minimal AtoMMC: just the part of the SD-card interface at `B400` that
//! programs use for the joystick.
//!
//! AtoMMC is a modern add-on that plugs an SD card into the Atom. The Atom
//! talks to it through two registers: it writes data to `B401`, writes a
//! command to `B400`, and reads the answer back from `B401`. Besides files,
//! the card's microcontroller exposes an 8-bit input port, and some games
//! (the Retro Software ports of Repton, Jet Set Willy and others) read their
//! joystick through it with the *read port* command.
//!
//! Only the port commands are emulated. Everything else (the SD card itself)
//! is not there, so those commands get no answer and the registers read as
//! open bus, exactly as when no AtoMMC is fitted. That also means programs
//! that only probe for an AtoMMC behave as before.
//!
//! Without an AtoMMC at all, the joystick read returned open bus (`B4`), which
//! such a game decodes as joystick right, left and up all pressed at once.

use super::memory::open_bus;

/// Commands (written to `B400`) for the microcontroller's spare port.
mod command {
    pub const GET_PORT_DDR: u8 = 0xA0;
    pub const SET_PORT_DDR: u8 = 0xA1;
    pub const READ_PORT: u8 = 0xA2;
    pub const WRITE_PORT: u8 = 0xA3;
}

/// The reply to a command that worked.
const STATUS_OK: u8 = 0x3F;

#[derive(Clone, Debug, Default)]
pub struct AtoMmc {
    /// The last byte written to the data register (`B401`).
    latch: u8,
    /// The reply to the last command we understand, until the next command.
    reply: Option<u8>,
    port_direction: u8,
    port_output: u8,
    /// The joystick's pins: bits clear when pressed. `None` = no joystick,
    /// which reads as 0 (as in Atomulator).
    pub joystick: Option<u8>,
}

impl AtoMmc {
    pub fn read(&self, addr: u16) -> u8 {
        self.reply.unwrap_or_else(|| open_bus(addr))
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        match addr & 0x0F {
            1 => self.latch = value,
            0 => self.run_command(value),
            _ => {}
        }
    }

    fn run_command(&mut self, command: u8) {
        self.reply = match command {
            command::GET_PORT_DDR => Some(self.port_direction),
            command::SET_PORT_DDR => {
                self.port_direction = self.latch;
                Some(STATUS_OK)
            }
            command::READ_PORT => Some(self.joystick.unwrap_or(0)),
            command::WRITE_PORT => {
                self.port_output = self.latch;
                Some(STATUS_OK)
            }
            _ => None, // the SD card is not emulated
        };
    }

    /// The RESET line clears the interface.
    pub fn reset(&mut self) {
        let joystick = self.joystick;
        *self = AtoMmc::default();
        self.joystick = joystick;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_as_open_bus_until_a_command_it_understands_is_given() {
        let mmc = AtoMmc::default();
        assert_eq!(mmc.read(0xB401), 0xB4);
        let mut mmc = AtoMmc::default();
        mmc.write(0xB400, 0x20); // an SD card command: not emulated
        assert_eq!(mmc.read(0xB401), 0xB4);
    }

    #[test]
    fn with_no_joystick_the_read_port_command_returns_zero() {
        // Games read this and treat 0 as "no joystick".
        let mut mmc = AtoMmc::default();
        mmc.write(0xB400, 0xA2);
        assert_eq!(mmc.read(0xB401), 0);
    }

    #[test]
    fn read_port_returns_the_joystick_pins() {
        let mut mmc = AtoMmc {
            joystick: Some(0b1111_1101),
            ..AtoMmc::default()
        };
        mmc.write(0xB400, 0xA2);
        assert_eq!(mmc.read(0xB401), 0b1111_1101);
    }

    #[test]
    fn port_direction_is_set_from_the_data_latch() {
        let mut mmc = AtoMmc::default();
        mmc.write(0xB401, 0xFF);
        mmc.write(0xB400, 0xA1);
        assert_eq!(mmc.read(0xB401), 0x3F, "STATUS_OK");
        mmc.write(0xB400, 0xA0);
        assert_eq!(mmc.read(0xB401), 0xFF, "the direction register reads back");
    }

    #[test]
    fn reset_clears_the_interface_but_keeps_the_joystick() {
        let mut mmc = AtoMmc {
            joystick: Some(5),
            ..AtoMmc::default()
        };
        mmc.write(0xB400, 0xA2);
        mmc.reset();
        assert_eq!(mmc.read(0xB401), 0xB4);
        assert_eq!(mmc.joystick, Some(5));
    }
}
