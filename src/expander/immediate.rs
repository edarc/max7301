//! Immediate-mode I/O adapter.

use core::marker::PhantomData;

use expander::pin::{ExpanderIO, PortPin};
use expander::Expander;
use interface::ExpanderInterface;
use mutex::IOMutex;
use registers::valid_port;

/// This I/O adapter captures the `Expander` and provides a factory for generating GPIO pins that
/// implement `InputPin` and `OutputPin` traits. Each such pin will immediately issue a bus
/// transaction to get or set the value every time any pin is accessed.
pub struct ImmediateIO<M, EI>(M, PhantomData<EI>)
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface + Send;

impl<M, EI> ImmediateIO<M, EI>
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface + Send,
{
    pub(crate) fn new(expander: Expander<EI>) -> Self {
        ImmediateIO(M::new(expander), PhantomData)
    }

    // cortex-m Mutex doesn't support this operation.
    // /// Release the `Expander` from this adapter, consuming the latter.
    // pub fn release(self) -> Expander<EI> {
    //     self.0.into_inner()
    // }

    /// Create a `PortPin` corresponding to one of the ports on the MAX7301. The returned `PortPin`
    /// implements `InputPin` and `OutputPin`, and using any of the methods from these traits on
    /// the returned `PortPin` will trigger a bus transaction to immediately read or write the
    /// value of that I/O port.
    pub fn port_pin<'io>(&'io self, port: u8) -> PortPin<'io, Self> {
        PortPin::new(self, valid_port(port))
    }
}

impl<M, EI> ExpanderIO for ImmediateIO<M, EI>
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface + Send,
{
    type Error = EI::Error;

    fn write_port(&self, port: u8, bit: bool) -> Result<(), EI::Error> {
        self.0.lock(|ex| ex.write_port(port, bit))
    }
    fn read_port(&self, port: u8) -> Result<bool, EI::Error> {
        self.0.lock(|ex| ex.read_port(port))
    }
}

#[cfg(test)]
mod tests {
    use expander::Expander;
    use hal::digital::v2::{InputPin, OutputPin};
    use interface::test_spy::{TestRegister as TR, TestSpyInterface};
    use mutex::DefaultMutex;

    #[test]
    fn single_pin_write() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_immediate::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);

        assert!(pin_twelve.set_high().is_ok());
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0x01));
    }

    #[test]
    fn single_pin_read() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_immediate::<DefaultMutex<_>>();
        let pin_twelve = io.port_pin(12);

        ei.set(0x2C, TR::ResetValue(0x00));
        assert_eq!(pin_twelve.is_high(), Ok(false));

        ei.set(0x2C, TR::ResetValue(0x01));
        assert_eq!(pin_twelve.is_high(), Ok(true));
    }

    #[test]
    fn multi_pin_read_write() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_immediate::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);
        let mut pin_sixteen = io.port_pin(16);
        let pin_twenty = io.port_pin(20);

        ei.set(0x34, TR::ResetValue(0x01));
        assert!(pin_twelve.set_high().is_ok());
        assert!(pin_sixteen.set_low().is_ok());
        assert_eq!(pin_twenty.is_low(), Ok(false));
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0x01));
        assert_eq!(ei.get(0x30), TR::WrittenValue(0x00));
    }
}
