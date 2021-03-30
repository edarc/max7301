//! The port expander device API. This provides the `Expander` type which is a direct abstraction
//! of the MAX7301. It allows direct use of all operations available on the device.

use config::{BankConfig, Configurator, ExpanderConfig};
use expander::immediate::ImmediateIO;
use expander::transactional::TransactionalIO;
use interface::ExpanderInterface;
use mutex::IOMutex;
use registers::Register;

pub mod immediate;
pub mod pin;
pub mod transactional;

/// The port expander device itself.
pub struct Expander<EI: ExpanderInterface> {
    iface: EI,
    pub(crate) config: ExpanderConfig,
}

impl<EI: ExpanderInterface + Send> Expander<EI> {
    /// Create a new `Expander`.
    ///
    /// Takes ownership of the `ExpanderInterface` which it should use to communicate with the
    /// MAX7301.
    pub fn new(iface: EI) -> Self {
        Self {
            iface,
            config: ExpanderConfig::default(),
        }
    }

    /// Begin (re)configuring the port expander hardware by returning a [`Configurator`].
    ///
    /// The `Configurator` is a builder-like interface that can be used to alter port modes and
    /// device configuration bits.
    pub fn configure<'e>(&'e mut self) -> Configurator<'e, EI> {
        Configurator::new(self)
    }

    /// Convert this expander into an immediate-mode I/O adapter.
    ///
    /// The I/O adapter can be used to generate individual `PortPin`s that allow
    /// `embedded-hal`-compatible access to the GPIOs on the expander directly, with every
    /// operation immediately triggering a bus operation.
    ///
    /// See [`ImmediateIO`] for detail.
    pub fn into_immediate<M: IOMutex<Self>>(self) -> ImmediateIO<M, EI> {
        ImmediateIO::new(self)
    }

    /// Convert this expander into a transactional I/O adapter.
    ///
    /// The I/O adapter can be used to generate individual `PortPin`s that allow
    /// `embedded-hal`-compatible access to the GPIOs on the expander. Unlike immediate mode, the
    /// operations on `PortPin` trait methods are buffered in a write-back cache.
    ///
    /// See [`TransactionalIO`] for detail.
    pub fn into_transactional<M: IOMutex<Self>>(self) -> TransactionalIO<M, EI> {
        TransactionalIO::new(self)
    }

    /// Perform a read of the current value of a single I/O port on the expander.
    pub fn read_port(&mut self, port: u8) -> Result<bool, ()> {
        self.iface
            .read_register(Register::SinglePort(port).into())
            .map(|v| v == 0x01)
    }

    /// Perform a read of the current value of 8 consecutive I/O ports on the expander in a single
    /// bus transaction.
    ///
    /// There is no alignment requirement; the `start_port` may be any valid port, and that port
    /// along with up to 7 following ports will be read in one transaction. The return value is a
    /// `u8` where the LSB is the value read from `start_port`, and each higher bit is the 7 ports
    /// following it in ascending order. If any of the bits would correspond to a port higher than
    /// 31, then those bits will be unset.
    pub fn read_ports(&mut self, start_port: u8) -> Result<u8, ()> {
        self.iface
            .read_register(Register::PortRange(start_port).into())
    }

    /// Write a value to a single I/O port on the expander.
    pub fn write_port(&mut self, port: u8, bit: bool) -> Result<(), ()> {
        self.iface.write_register(
            Register::SinglePort(port).into(),
            if bit { 0x01 } else { 0x00 },
        )
    }

    /// Write a value to 8 consecutive I/O ports on the expander in a single bus transaction.
    ///
    /// There is no alignment requirement; the `start_port` may be any valid port, and that port
    /// along with up to 7 following ports will be written in one transaction. `bits` is a `u8`
    /// where the LSB is the value to write to `start_port`, and each higher bit is the 7 ports
    /// following it in ascending order. If any of the bits would correspond to a port higher than
    /// 31, then those bits will be ignored.
    pub fn write_ports(&mut self, start_port: u8, bits: u8) -> Result<(), ()> {
        self.iface
            .write_register(Register::PortRange(start_port).into(), bits)
    }

    pub(crate) fn write_config(&mut self) -> Result<(), ()> {
        self.iface
            .write_register(Register::Configuration.into(), self.config.clone().into())
    }

    pub(crate) fn write_bank_config(&mut self, bank: u8, cfg: BankConfig) -> Result<(), ()> {
        self.iface
            .write_register(Register::BankConfig(bank).into(), cfg.into())
    }

    pub(crate) fn read_modify_bank_config(
        &mut self,
        bank: u8,
        f: impl Fn(u8) -> BankConfig,
    ) -> Result<(), ()> {
        let addr = Register::BankConfig(bank).into();
        let current = self.iface.read_register(addr)?;
        self.iface.write_register(addr, f(current).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::PortMode;
    use interface::test_spy::{TestRegister as TR, TestSpyInterface};

    #[test]
    fn expander_configure_noop() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex.configure().commit().is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![TR::ResetValue(0b10101010); 7]
        );
    }

    #[test]
    fn expander_configure_shutdown() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex.configure().shutdown(false).commit().is_ok());
        assert_eq!(ei.get(0x04), TR::WrittenValue(0b00000001));
    }

    #[test]
    fn expander_configure_detect_transitions() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex.configure().detect_transitions(true).commit().is_ok());
        assert_eq!(ei.get(0x04), TR::WrittenValue(0b10000000));
    }

    #[test]
    fn expander_configure_port_single_read_modify() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex.configure().port(4, PortMode::Output).commit().is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![
                TR::WrittenValue(0b10101001),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
            ]
        );
        assert_eq!(ei.reads(), vec![0x09]);
    }

    #[test]
    fn expander_configure_ports_read_modify() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .configure()
            .ports(4..=6, PortMode::Output)
            .commit()
            .is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![
                TR::WrittenValue(0b10010101),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
            ]
        );
        assert_eq!(ei.reads(), vec![0x09]);
    }

    #[test]
    fn expander_configure_ports_overwrite() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .configure()
            .ports(4..=7, PortMode::Output)
            .commit()
            .is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![
                TR::WrittenValue(0b01010101),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
            ]
        );
        assert_eq!(ei.reads(), vec![]);
    }

    #[test]
    fn expander_configure_ports_spanning() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .configure()
            .ports(6..=13, PortMode::Output)
            .commit()
            .is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![
                TR::WrittenValue(0b01011010),
                TR::WrittenValue(0b01010101),
                TR::WrittenValue(0b10100101),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
            ]
        );
        assert_eq!(ei.reads(), vec![0x09, 0x0B]);
    }

    #[test]
    fn expander_configure_ports_port_overlapping() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .configure()
            .ports(4..=11, PortMode::Output)
            .port(7, PortMode::InputPullup)
            .commit()
            .is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![
                TR::WrittenValue(0b11010101),
                TR::WrittenValue(0b01010101),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
                TR::ResetValue(0b10101010),
            ]
        );
        assert_eq!(ei.reads(), vec![]);
    }

    #[test]
    fn expander_configure_ports_and_shutdown() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .configure()
            .ports(4..=7, PortMode::Output)
            .shutdown(false)
            .commit()
            .is_ok());
        assert_eq!(ei.get(0x04), TR::WrittenValue(0b00000001));
        assert_eq!(ei.get(0x09), TR::WrittenValue(0b01010101),);
    }
}
