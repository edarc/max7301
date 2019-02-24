//! This module provide shims for the `embedded-hal` hardware correspoding to the MAX7301's
//! supported electrical/bus interfaces. It is a shim between `embedded-hal` implementations and
//! the expander's registers.

use registers::RegisterAddress;

/// An interface for the MAX7301 implements this trait, which provides the basic operations for
/// sending pre-encoded register accesses to the chip via the interface.
pub trait ExpanderInterface {
    /// Issue a write command to the expander to write `value` into the register at `addr`.
    fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), ()>;
    /// Issue a read command to the expander to fetch the `u8` value at register `addr`.
    fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, ()>;
}

// This is here (and has to be pub) for doctests only. It's useless otherwise.
#[doc(hidden)]
pub mod noop {
    use super::ExpanderInterface;
    use registers::RegisterAddress;
    pub struct NoopInterface;
    impl ExpanderInterface for NoopInterface {
        fn write_register(&mut self, _addr: RegisterAddress, _value: u8) -> Result<(), ()> {
            Ok(())
        }
        fn read_register(&mut self, _addr: RegisterAddress) -> Result<u8, ()> {
            Ok(0u8)
        }
    }
}

pub mod spi {
    //! The SPI interface controls a MAX7301 via a 4-wire interface (SCK, MOSI, MISO, CS).

    use hal;

    use super::{ExpanderInterface, RegisterAddress};
    use registers::Register;

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
        CS: hal::digital::OutputPin,
    {
        /// Create a new SPI interface to communicate with the port expander. `spi` is the SPI
        /// master device, and `cs` is the GPIO output pin connected to the CS pin of the MAX7301.
        pub fn new(spi: SPI, mut cs: CS) -> Self {
            cs.set_high();
            Self { spi, cs }
        }
    }

    impl<SPI, CS> ExpanderInterface for SpiInterface<SPI, CS>
    where
        SPI: hal::blocking::spi::Write<u8> + hal::blocking::spi::Transfer<u8>,
        CS: hal::digital::OutputPin,
    {
        fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), ()> {
            // Address goes in upper byte, value goes in lower. Address MSB is zero for a write.
            let buf = [u8::from(addr), value];

            // Select chip and do bus write.
            self.cs.set_low();
            let result = self.spi.write(&buf);
            self.cs.set_high();

            match result {
                Ok(()) => Ok(()),
                Err(_) => Err(()),
            }
        }

        fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, ()> {
            // Address goes in upper byte, lower byte is don't-care because it will be clobbered
            // when CS goes high. Address MSB is *set* for a read.
            let addr_word = 0x80 | u8::from(addr);

            // Select chip and do bus write.
            self.cs.set_low();
            let addr_result = self.spi.write(&[addr_word, 0]);
            self.cs.set_high();

            match addr_result {
                Err(_) => return Err(()),
                _ => {}
            };

            // Expander has latched the value of the requested register into the low byte of its
            // SPI shift register at CS rising edge. Select chip again and shift it back to us.
            // Shift in a no-op so the expander will do nothing on second CS falling edge.
            let mut buf = [RegisterAddress::from(Register::Noop).into(), 0u8];
            self.cs.set_low();
            let data_result = self.spi.transfer(&mut buf);
            self.cs.set_high();

            match data_result {
                Err(_) => Err(()),
                Ok(buf) => {
                    if buf[0] != addr_word {
                        Err(())
                    } else {
                        Ok(buf[1])
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_spy {
    //! An interface for use in unit tests to spy on whatever was sent to it.

    use super::ExpanderInterface;
    use registers::RegisterAddress;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum TestRegister {
        Forbidden,
        IgnoreWrite(u8),
        ResetValue(u8),
        WrittenValue(u8),
    }

    pub struct TestSpyInterface {
        registers: Rc<RefCell<Vec<TestRegister>>>,
        reads: Rc<RefCell<Vec<u8>>>,
    }

    impl TestSpyInterface {
        pub fn new() -> Self {
            let mut new = Self {
                registers: Rc::new(RefCell::new(Vec::new())),
                reads: Rc::new(RefCell::new(Vec::new())),
            };
            new.reset();
            new
        }

        pub fn reset(&mut self) {
            use self::TestRegister::*;

            self.reads.borrow_mut().clear();
            let mut regs = self.registers.borrow_mut();
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
            self.registers.borrow()[addr as usize]
        }

        pub fn set(&mut self, addr: u8, val: TestRegister) {
            self.registers.borrow_mut()[addr as usize] = val;
        }

        pub fn reads(&self) -> Vec<u8> {
            self.reads.borrow().clone()
        }
    }

    impl ExpanderInterface for TestSpyInterface {
        fn write_register(&mut self, addr: RegisterAddress, value: u8) -> Result<(), ()> {
            let mut regs = self.registers.borrow_mut();
            let enc_addr = u8::from(addr);
            assert!(enc_addr <= 0x5F);
            match regs[enc_addr as usize] {
                TestRegister::Forbidden => panic!("Write to forbidden register {}", enc_addr),
                TestRegister::IgnoreWrite(_) => {}
                ref mut m => *m = TestRegister::WrittenValue(value),
            };
            Ok(())
        }
        fn read_register(&mut self, addr: RegisterAddress) -> Result<u8, ()> {
            self.reads.borrow_mut().push(addr.into());
            let regs = self.registers.borrow();
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
}
