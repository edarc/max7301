//! Immediate-mode I/O adapter.

use core::cell::RefCell;

use expander::pin::{ExpanderIO, Pin};
use expander::Expander;
use interface::ExpanderInterface;
use registers::valid_port;

/// This I/O adapter captures the `Expander` and provides a factory for generating GPIO pins that
/// implement `InputPin` and `OutputPin` traits. Each such pin will immediately issue a bus
/// transaction to get or set the value every time any pin is accessed.
pub struct ImmediateIO<EI: ExpanderInterface>(RefCell<Expander<EI>>);

impl<EI: ExpanderInterface> ImmediateIO<EI> {
    pub(crate) fn new(expander: Expander<EI>) -> Self {
        ImmediateIO(RefCell::new(expander))
    }

    /// Release the `Expander` from this adapter, consuming the latter.
    pub fn release(self) -> Expander<EI> {
        self.0.into_inner()
    }

    /// Create a `Pin` corresponding to one of the ports on the MAX7301. The returned `Pin`
    /// implements `InputPin` and `OutputPin`, and using any of the methods from these traits on
    /// the returned `Pin` will trigger a bus transaction to immediately read or write the value of
    /// that I/O port.
    pub fn port_pin<'io>(&'io self, port: u8) -> Pin<'io, Self> {
        Pin::new(self, valid_port(port))
    }
}

impl<EI: ExpanderInterface> ExpanderIO for ImmediateIO<EI> {
    fn write_port(&self, port: u8, bit: bool) {
        self.0.borrow_mut().write_port(port, bit).unwrap()
    }
    fn read_port(&self, port: u8) -> bool {
        self.0.borrow_mut().read_port(port).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expander::Expander;
    use hal::digital::{InputPin, OutputPin};
    use interface::test_spy::{TestRegister as TR, TestSpyInterface};

    #[test]
    fn single_pin_write() {
        let ei = TestSpyInterface::new();
        let io = ImmediateIO::new(Expander::new(ei.split()));
        let mut pin_twelve = io.port_pin(12);

        pin_twelve.set_high();
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0x01));
    }

    #[test]
    fn single_pin_read() {
        let mut ei = TestSpyInterface::new();
        let io = ImmediateIO::new(Expander::new(ei.split()));
        let pin_twelve = io.port_pin(12);

        ei.set(0x2C, TR::ResetValue(0x00));
        assert_eq!(pin_twelve.is_high(), false);

        ei.set(0x2C, TR::ResetValue(0x01));
        assert_eq!(pin_twelve.is_high(), true);
    }

    #[test]
    fn multi_pin_read_write() {
        let mut ei = TestSpyInterface::new();
        let io = ImmediateIO::new(Expander::new(ei.split()));
        let mut pin_twelve = io.port_pin(12);
        let mut pin_sixteen = io.port_pin(16);
        let pin_twenty = io.port_pin(20);

        ei.set(0x34, TR::ResetValue(0x01));
        pin_twelve.set_high();
        pin_sixteen.set_low();
        assert_eq!(pin_twenty.is_low(), false);
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0x01));
        assert_eq!(ei.get(0x30), TR::WrittenValue(0x00));
    }
}
