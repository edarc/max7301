//! APIs for interacting with I/O pins on the MAX7301 through an `embedded-hal` API.

#[cfg(feature = "unproven")]
use hal::digital::InputPin;
use hal::digital::OutputPin;

/// An indirection between I/O pin abstractions and the expander itself, which allows selection
/// between transactional reads and writes, which reduce bus traffic and latency, and
/// immediate-mode reads and writes, which add more bus traffic and latency but are simpler to use.
pub trait ExpanderIO {
    /// Write the value of an I/O port. `port` is a port number between 4 and 31; `bit` is the
    /// value to set the port to. If the pin is configured as an output, the value (`true` is
    /// logic high, `false` logic low) will be asserted on the corresponding pin.
    fn write_port(&self, port: u8, bit: bool);

    /// Read the value of an I/O port. `port` is a port number between 4 and 31, the value of that
    /// pin will be returned (`false` if logic low, `true` if logic high). If the pin is configured
    /// as an output, the last set value will be read; if it is configured as an input, the
    /// logic level of the externally applied signal will be read.
    fn read_port(&self, port: u8) -> bool;
}

/// A single I/O pin on the MAX7301. These implement the `embedded-hal` traits for GPIO pins, so
/// they can be used to transparently connect devices driven over GPIOs through the MAX7301
/// instead, using their `embedded-hal`-compatible drivers without modification.
pub struct PortPin<'io, IO: ExpanderIO> {
    io: &'io IO,
    port: u8,
}

impl<'io, IO: ExpanderIO> PortPin<'io, IO> {
    pub(crate) fn new(io: &'io IO, port: u8) -> Self {
        Self { io, port }
    }
}

impl<'io, IO: ExpanderIO> OutputPin for PortPin<'io, IO> {
    fn set_high(&mut self) {
        self.io.write_port(self.port, true)
    }
    fn set_low(&mut self) {
        self.io.write_port(self.port, false)
    }
}

#[cfg(feature = "unproven")]
impl<'io, IO: ExpanderIO> InputPin for PortPin<'io, IO> {
    fn is_high(&self) -> bool {
        self.io.read_port(self.port)
    }
    fn is_low(&self) -> bool {
        !self.io.read_port(self.port)
    }
}
