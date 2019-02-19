//! The port expander device API.

use config::{BankConfig, ExpanderConfig, PortConfigurator};
use interface::ExpanderInterface;
use registers::Register;

pub struct Expander<EI: ExpanderInterface> {
    iface: EI,
}

impl<EI: ExpanderInterface> Expander<EI> {
    pub fn new(iface: EI) -> Self {
        Self { iface }
    }

    pub fn write_config(&mut self, cfg: ExpanderConfig) -> Result<(), ()> {
        self.iface
            .write_register(Register::Configuration.into(), cfg.into())
    }

    pub fn reconfigure_ports<'e>(&'e mut self) -> PortConfigurator<'e, EI> {
        PortConfigurator::new(self)
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
    use config::PortConfig;
    use interface::test_spy::{TestRegister as TR, TestSpyInterface};

    #[test]
    fn expander_write_config() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .write_config(ExpanderConfig {
                shutdown: false,
                transition_detect: false,
            })
            .is_ok());
        assert_eq!(ei.get(0x04), TR::WrittenValue(0b00000000));
    }

    #[test]
    fn expander_reconfigure_ports_noop() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex.reconfigure_ports().commit().is_ok());
        assert_eq!(
            (0x09..0x10)
                .into_iter()
                .map(|a| ei.get(a))
                .collect::<Vec<_>>(),
            vec![TR::ResetValue(0b10101010); 7]
        );
    }

    #[test]
    fn expander_reconfigure_ports_single_read_modify() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .reconfigure_ports()
            .with_port(4, PortConfig::Output)
            .commit()
            .is_ok());
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
    fn expander_reconfigure_ports_range_read_modify() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .reconfigure_ports()
            .with_ports(4..=6, PortConfig::Output)
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
    fn expander_reconfigure_ports_range_overwrite() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .reconfigure_ports()
            .with_ports(4..=7, PortConfig::Output)
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
    fn expander_reconfigure_ports_range_spanning() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .reconfigure_ports()
            .with_ports(6..=13, PortConfig::Output)
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
    fn expander_reconfigure_ports_range_overlapping() {
        let ei = TestSpyInterface::new();
        let mut ex = Expander::new(ei.split());
        assert!(ex
            .reconfigure_ports()
            .with_ports(4..=11, PortConfig::Output)
            .with_port(7, PortConfig::InputPullup)
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
}
