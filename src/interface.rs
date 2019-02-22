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

//. A `UnitAddress` specifies the address of the MAX7301 on the bus interface. For SPI units,
//. this allows supporting multiple MAX7301s on a single CS, with their shift registers chained
//. together using the method documented in the datasheet. (In the future, this driver may be
//. extended to support the MAX7300 which is the I2C variant, in which case this specifies the
//. I2C address).
// type UnitAddress;

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
