//! This module provide shims for the `embedded-hal` hardware correspoding to the MAX7301's
//! supported electrical/bus interfaces. It is a shim between `embedded-hal` implementations and
//! the expander's registers.

use registers::RegisterAddress;

/// An interface for the MAX7301 implements this trait, which provides the basic operations for
/// sending pre-encoded register accesses to the chip via the interface.
pub trait ExpanderInterface {
    /// The type of error that register reads and writes may return.
    type Error;
    /// Issue a write command to the expander to write `value` into the register at `addr`.
    fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), Self::Error>;
    /// Issue a read command to the expander to fetch the `u8` value at register `addr`.
    fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, Self::Error>;
}

// This is here (and has to be pub) for doctests only. It's useless otherwise.
#[doc(hidden)]
pub mod noop {
    use super::ExpanderInterface;
    use registers::RegisterAddress;
    pub struct NoopInterface;
    impl ExpanderInterface for NoopInterface {
        type Error = core::convert::Infallible;
        fn write_register(
            &mut self,
            _addr: RegisterAddress,
            _value: u8,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
        fn read_register(&mut self, _addr: RegisterAddress) -> Result<u8, Self::Error> {
            Ok(0u8)
        }
    }
}

pub mod spi {
    //! The SPI interface controls a MAX7301 via a 4-wire interface (SCK, MOSI, MISO, CS).

    use hal;

    use super::{ExpanderInterface, RegisterAddress};
    use registers::Register;

    /// The union of all errors that may occur on the SPI interface. This primarily consists of
    /// variants for each of the error types for the chip select GPIO, SPI write, and SPI transfer.
    #[derive(Debug)]
    pub enum SpiInterfaceError<CSE, WE, TE> {
        /// The chip select GPIO threw an error.
        CSError(CSE),
        /// An error occurred during SPI write.
        WriteError(WE),
        /// An error occurred during SPI transfer.
        TransferError(TE),
        /// A register address was returned by the device that does not match what was sent. This
        /// is probably a hardware issue.
        AddressError,
    }

    impl<CSE, WE, TE> SpiInterfaceError<CSE, WE, TE> {
        fn from_cs(e: CSE) -> Self {
            Self::CSError(e)
        }
        fn from_write(e: WE) -> Self {
            Self::WriteError(e)
        }
        fn from_transfer(e: TE) -> Self {
            Self::TransferError(e)
        }
    }

    /// A configured `ExpanderInterface` for controlling a MAX7301 via SPI.
    pub struct SpiInterface<SPI, CS> {
        /// The SPI master device connected to the MAX7301.
        spi: SPI,
        /// A GPIO output pin connected to the CS pin of the MAX7301.
        cs: CS,
    }

    impl<SPI, CS> SpiInterface<SPI, CS>
    where
        SPI: hal::blocking::spi::Write<u8> + hal::blocking::spi::Transfer<u8>,
        CS: hal::digital::v2::OutputPin,
    {
        /// Create a new SPI interface to communicate with the port expander. `spi` is the SPI
        /// master device, and `cs` is the GPIO output pin connected to the CS pin of the MAX7301.
        pub fn new(spi: SPI, cs: CS) -> Self {
            Self { spi, cs }
        }
    }

    impl<SPI, CS> ExpanderInterface for SpiInterface<SPI, CS>
    where
        SPI: hal::blocking::spi::Write<u8> + hal::blocking::spi::Transfer<u8>,
        CS: hal::digital::v2::OutputPin,
    {
        type Error = SpiInterfaceError<
            <CS as hal::digital::v2::OutputPin>::Error,
            <SPI as hal::blocking::spi::Write<u8>>::Error,
            <SPI as hal::blocking::spi::Transfer<u8>>::Error,
        >;

        fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), Self::Error> {
            // Address goes in upper byte, value goes in lower. Address MSB is zero for a write.
            let buf = [u8::from(addr), value];

            // Select chip and do bus write.
            self.cs.set_low().map_err(Self::Error::from_cs)?;
            let result = self.spi.write(&buf);
            self.cs.set_high().map_err(Self::Error::from_cs)?;
            result.map_err(Self::Error::from_write)
        }

        fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, Self::Error> {
            // Address goes in upper byte, lower byte is don't-care because it will be clobbered
            // when CS goes high. Address MSB is *set* for a read.
            let addr_word = 0x80 | u8::from(addr);

            // Select chip and do bus write.
            self.cs.set_low().map_err(Self::Error::from_cs)?;
            let addr_result = self.spi.write(&[addr_word, 0]);
            self.cs.set_high().map_err(Self::Error::from_cs)?;
            addr_result.map_err(Self::Error::from_write)?;

            // Expander has latched the value of the requested register into the low byte of its
            // SPI shift register at CS rising edge. Select chip again and shift it back to us.
            // Shift in a no-op so the expander will do nothing on second CS falling edge.
            let mut buf = [RegisterAddress::from(Register::Noop).into(), 0u8];
            self.cs.set_low().map_err(Self::Error::from_cs)?;
            let data_result = self.spi.transfer(&mut buf);
            self.cs.set_high().map_err(Self::Error::from_cs)?;
            let return_data = data_result.map_err(Self::Error::from_transfer)?;

            if return_data[0] != addr_word {
                Err(Self::Error::AddressError)
            } else {
                Ok(return_data[1])
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_spy {
    //! An interface for use in unit tests to spy on whatever was sent to it.

    use super::ExpanderInterface;
    use registers::RegisterAddress;
    use std::fmt;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum TestRegister {
        Forbidden,
        IgnoreWrite(u8),
        ResetValue(u8),
        WrittenValue(u8),
    }

    pub struct TestSpyInterface {
        registers: Arc<Mutex<Vec<TestRegister>>>,
        reads: Arc<Mutex<Vec<u8>>>,
    }

    impl TestSpyInterface {
        pub fn new() -> Self {
            let mut new = Self {
                registers: Arc::new(Mutex::new(Vec::new())),
                reads: Arc::new(Mutex::new(Vec::new())),
            };
            new.reset();
            new
        }

        pub fn reset(&mut self) {
            use self::TestRegister::*;

            self.reads.lock().unwrap().clear();
            let mut regs = self.registers.lock().unwrap();
            regs.clear();
            regs.resize(0x60, Forbidden);

            regs[0x00] = IgnoreWrite(0x00); // No-op
            regs[0x04] = ResetValue(0b00000001); // Configuration
            regs[0x06] = ResetValue(0x00); // Transition detect mask

            // Bank configurations
            for addr in 0x09..=0x0F {
                regs[addr] = ResetValue(0b10101010);
            }

            // GPIO registers
            for addr in 0x20..=0x5F {
                regs[addr] = ResetValue(0x00);
            }
        }

        pub fn split(&self) -> Self {
            Self {
                registers: self.registers.clone(),
                reads: self.reads.clone(),
            }
        }

        pub fn get(&self, addr: u8) -> TestRegister {
            self.registers.lock().unwrap()[addr as usize]
        }

        pub fn set(&mut self, addr: u8, val: TestRegister) {
            self.registers.lock().unwrap()[addr as usize] = val;
        }

        pub fn reads(&self) -> Vec<u8> {
            self.reads.lock().unwrap().clone()
        }
    }

    impl ExpanderInterface for TestSpyInterface {
        type Error = std::convert::Infallible;

        fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), Self::Error> {
            let mut regs = self.registers.lock().unwrap();
            let enc_addr = u8::from(addr);
            assert!(enc_addr <= 0x5F);
            match regs[enc_addr as usize] {
                TestRegister::Forbidden => panic!("Write to forbidden register {}", enc_addr),
                TestRegister::IgnoreWrite(_) => {}
                ref mut m => *m = TestRegister::WrittenValue(value),
            };
            Ok(())
        }
        fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, Self::Error> {
            self.reads.lock().unwrap().push(addr.into());
            let regs = self.registers.lock().unwrap();
            let enc_addr = u8::from(addr);
            assert!(enc_addr <= 0x5F);
            match regs[enc_addr as usize] {
                TestRegister::Forbidden => panic!("Read from forbidden register {}", enc_addr),
                TestRegister::IgnoreWrite(v) => Ok(v),
                TestRegister::ResetValue(v) => Ok(v),
                TestRegister::WrittenValue(v) => Ok(v),
            }
        }
    }

    #[derive(Copy, Clone, PartialEq)]
    pub enum TestPort {
        Reset(bool),
        Read(bool),
        BlindWrite(bool),
        ReadWrite(bool),
    }

    impl fmt::Debug for TestPort {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            use self::TestPort::*;
            let (bit, status) = match *self {
                Reset(b) => (b, '_'),
                Read(b) => (b, 'r'),
                BlindWrite(b) => (b, 'X'),
                ReadWrite(b) => (b, 'w'),
            };
            let bit_fmt = if bit { '1' } else { '0' };
            write!(f, "{}{}", status, bit_fmt)
        }
    }

    pub struct SemanticTestSpyInterface {
        ports: Arc<Mutex<Vec<TestPort>>>,
    }

    impl SemanticTestSpyInterface {
        pub fn new(init: Vec<bool>) -> Self {
            assert!(init.len() == 32 - 4);
            Self {
                ports: Arc::new(Mutex::new(
                    init.into_iter()
                        .map(|b| TestPort::Reset(b))
                        .collect::<Vec<_>>(),
                )),
            }
        }

        pub fn split(&self) -> Self {
            Self {
                ports: self.ports.clone(),
            }
        }

        pub fn peek_all(&self) -> Vec<TestPort> {
            use self::TestPort::*;
            self.ports
                .lock()
                .unwrap()
                .iter()
                .cloned()
                .map(|v| match v {
                    Read(b) => Reset(b),
                    other => other,
                })
                .collect()
        }

        pub fn peek_bits(&self) -> Vec<bool> {
            use self::TestPort::*;
            self.ports
                .lock()
                .unwrap()
                .iter()
                .cloned()
                .map(|v| match v {
                    Reset(b) | Read(b) | BlindWrite(b) | ReadWrite(b) => b,
                })
                .collect()
        }

        fn write_port(&self, port: u8, bit: bool) {
            use self::TestPort::*;
            let idx = port as usize - 4;
            let slot_ref = &mut self.ports.lock().unwrap()[idx];
            *slot_ref = match slot_ref {
                Reset(_) | BlindWrite(_) => BlindWrite(bit),
                Read(_) | ReadWrite(_) => ReadWrite(bit),
            };
        }

        fn read_port(&self, port: u8) -> bool {
            use self::TestPort::*;
            let idx = port as usize - 4;
            let slot_ref = &mut self.ports.lock().unwrap()[idx];
            let (upd, ret) = match slot_ref {
                Reset(b) | Read(b) => (Read(*b), *b),
                ReadWrite(b) => (ReadWrite(*b), *b),
                BlindWrite(b) => (BlindWrite(*b), *b),
            };
            *slot_ref = upd;
            ret
        }
    }

    impl fmt::Debug for SemanticTestSpyInterface {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(
                f,
                "[ {} ]",
                self.ports
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|&v| format!("{:?}", v))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    }

    impl ExpanderInterface for SemanticTestSpyInterface {
        type Error = std::convert::Infallible;

        fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), Self::Error> {
            match u8::from(addr) {
                single @ 0x24..=0x3F => {
                    let port = single - 0x20;
                    self.write_port(port, (value & 0x01) == 1);
                }
                multi @ 0x44..=0x5F => {
                    let start_port = multi - 0x40;
                    for port_offset in 0..8 {
                        let port = start_port + port_offset;
                        if port <= 31 {
                            self.write_port(port, (value & 1 << port_offset) != 0);
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, Self::Error> {
            Ok(match u8::from(addr) {
                single @ 0x24..=0x3F => {
                    let port = single - 0x20;
                    if self.read_port(port) {
                        1
                    } else {
                        0
                    }
                }
                multi @ 0x44..=0x5F => {
                    let start_port = multi - 0x40;
                    let mut bits = 0u8;
                    for port_offset in 0..8 {
                        let port = start_port + port_offset;
                        if port <= 31 {
                            bits |= if self.read_port(port) {
                                1 << port_offset
                            } else {
                                0
                            }
                        }
                    }
                    bits
                }
                _ => 0,
            })
        }
    }
}
