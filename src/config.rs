//! Configuration abstractions.

use expander::Expander;
use interface::ExpanderInterface;
use registers::valid_port;

fn port_bank_and_offset(port: u8) -> (u8, u8) {
    (valid_port(port) / 4 - 1, port % 4)
}

#[derive(Clone, Copy, Debug)]
pub enum PortMode {
    Output,
    InputFloating,
    InputPullup,
}

impl From<PortMode> for u8 {
    fn from(cfg: PortMode) -> u8 {
        use self::PortMode::*;
        match cfg {
            Output => 0b01,
            InputFloating => 0b10,
            InputPullup => 0b11,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BankConfig(u8);

#[derive(Clone, Copy, Debug, PartialEq)]
enum BankConfigStatus {
    Unchanged,
    ReadModify,
    Overwrite,
}

impl Default for BankConfig {
    fn default() -> Self {
        Self(0)
    }
}

impl BankConfig {
    fn set_port(&mut self, port_offset: u8, cfg: PortMode) {
        match port_offset {
            0...4 => {
                let mask = !(0b11u8 << port_offset * 2);
                let cfg_bits = u8::from(cfg) << port_offset * 2;
                self.0 = self.0 & mask | cfg_bits;
            }
            _ => panic!("Config register can only hold 4 ports"),
        }
    }
    fn keep_mask(&self) -> u8 {
        (0..4)
            .into_iter()
            .map(|p| {
                if self.0 & (0b11u8 << p * 2) == 0 {
                    0b11u8 << p * 2
                } else {
                    0u8
                }
            })
            .fold(0u8, |m, a| a | m)
    }
    fn status(&self) -> BankConfigStatus {
        match self.keep_mask() {
            0xFF => BankConfigStatus::Unchanged,
            0x00 => BankConfigStatus::Overwrite,
            _ => BankConfigStatus::ReadModify,
        }
    }
    fn merge(&self, current: u8) -> Self {
        Self(current & self.keep_mask() | self.0)
    }
}

impl From<BankConfig> for u8 {
    fn from(cfg: BankConfig) -> u8 {
        cfg.0
    }
}

#[derive(Clone)]
pub(crate) struct ExpanderConfig {
    pub shutdown: bool,
    pub transition_detect: bool,
}

impl Default for ExpanderConfig {
    fn default() -> Self {
        Self {
            shutdown: true,
            transition_detect: false,
        }
    }
}

impl From<ExpanderConfig> for u8 {
    fn from(cfg: ExpanderConfig) -> u8 {
        let shtd = if cfg.shutdown { 0b00000001 } else { 0 };
        let txnd = if cfg.transition_detect { 0b10000000 } else { 0 };
        shtd | txnd
    }
}

#[must_use = "Configuration changes are not applied unless committed"]
pub struct Configurator<'e, EI: ExpanderInterface> {
    expander: &'e mut Expander<EI>,
    expander_config_dirty: bool,
    banks: [BankConfig; 7],
}

impl<'e, EI: ExpanderInterface> Configurator<'e, EI> {
    pub(crate) fn new(expander: &'e mut Expander<EI>) -> Self {
        Self {
            expander,
            expander_config_dirty: false,
            banks: [BankConfig(0); 7],
        }
    }

    fn set_port(&mut self, port: u8, cfg: PortMode) {
        let (bank, offset) = port_bank_and_offset(port);
        self.banks[bank as usize].set_port(offset, cfg);
    }

    pub fn port(mut self, port: u8, cfg: PortMode) -> Self {
        self.set_port(port, cfg);
        self
    }

    pub fn ports<I>(mut self, ports: I, cfg: PortMode) -> Self
    where
        I: IntoIterator<Item = u8>,
    {
        for port in ports {
            self.set_port(port, cfg);
        }
        self
    }

    pub fn shutdown(mut self, enable: bool) -> Self {
        self.expander.config.shutdown = enable;
        self.expander_config_dirty = true;
        self
    }

    pub fn detect_transitions(mut self, enable: bool) -> Self {
        self.expander.config.transition_detect = enable;
        self.expander_config_dirty = true;
        self
    }

    pub fn commit(self) -> Result<(), ()> {
        for (bank, bank_config) in self.banks.iter().enumerate() {
            match bank_config.status() {
                BankConfigStatus::Unchanged => {}
                BankConfigStatus::Overwrite => {
                    self.expander.write_bank_config(bank as u8, *bank_config)?;
                }
                BankConfigStatus::ReadModify => {
                    self.expander
                        .read_modify_bank_config(bank as u8, |cur| bank_config.merge(cur))?;
                }
            }
        }
        if self.expander_config_dirty {
            self.expander.write_config()
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_config_set_port_valid() {
        let mut bank = BankConfig::default();
        bank.set_port(0, PortMode::InputPullup);
        bank.set_port(2, PortMode::Output);
        assert_eq!(u8::from(bank), 0b00010011);
    }

    #[test]
    #[should_panic]
    fn bank_config_set_port_invalid() {
        let mut bank = BankConfig::default();
        bank.set_port(4, PortMode::InputPullup);
    }

    #[test]
    fn bank_config_keep_mask_unchanged() {
        let bank = BankConfig::default();
        assert_eq!(bank.keep_mask(), 0b11111111);
        assert_eq!(bank.status(), BankConfigStatus::Unchanged);
    }

    #[test]
    fn bank_config_keep_mask_change_0() {
        let mut bank = BankConfig::default();
        bank.set_port(0, PortMode::InputPullup);
        assert_eq!(bank.keep_mask(), 0b11111100);
        assert_eq!(bank.status(), BankConfigStatus::ReadModify);
    }

    #[test]
    fn bank_config_keep_mask_change_0_2() {
        let mut bank = BankConfig::default();
        bank.set_port(0, PortMode::InputPullup);
        bank.set_port(2, PortMode::Output);
        assert_eq!(bank.keep_mask(), 0b11001100);
        assert_eq!(bank.status(), BankConfigStatus::ReadModify);
    }

    #[test]
    fn bank_config_keep_mask_change_all() {
        let mut bank = BankConfig::default();
        for p in 0..4 {
            bank.set_port(p, PortMode::Output);
        }
        assert_eq!(bank.keep_mask(), 0b00000000);
        assert_eq!(bank.status(), BankConfigStatus::Overwrite);
    }

    #[test]
    fn bank_config_merge() {
        let orig = 0b11101010u8;
        let mut bank = BankConfig::default();
        bank.set_port(0, PortMode::InputPullup);
        bank.set_port(2, PortMode::Output);
        assert_eq!(u8::from(bank.merge(orig)), 0b11011011u8);
    }

    #[test]
    fn expander_config_default() {
        let expander_config = ExpanderConfig::default();
        assert_eq!(u8::from(expander_config), 0b00000001);
    }

    #[test]
    fn expander_config_disable_shutdown() {
        let mut expander_config = ExpanderConfig::default();
        expander_config.shutdown = false;
        assert_eq!(u8::from(expander_config), 0b00000000);
    }

    #[test]
    fn expander_config_enable_transition_detect() {
        let mut expander_config = ExpanderConfig::default();
        expander_config.transition_detect = true;
        assert_eq!(u8::from(expander_config), 0b10000001);
    }
}
