//! Abstractions used to configure the MAX7301 hardware.

use expander::Expander;
use interface::ExpanderInterface;
use registers::valid_port;

fn port_bank_and_offset(port: u8) -> (u8, u8) {
    (valid_port(port) / 4 - 1, port % 4)
}

/// A `PortMode` enumerates the three supported modes that each GPIO pin on the MAX7301 may be
/// configured to.
#[derive(Clone, Copy, Debug)]
pub enum PortMode {
    /// Push-pull logic output.
    Output,
    /// Floating logic input.
    InputFloating,
    /// Logic input with weak pull-up.
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
            0..=4 => {
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
    shutdown: bool,
    transition_detect: bool,
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
        let shtd = if cfg.shutdown { 0 } else { 0b00000001 };
        let txnd = if cfg.transition_detect { 0b10000000 } else { 0 };
        shtd | txnd
    }
}

/// A `Configurator` provides methods to build a list of device configuration changes, such as port
/// modes and device configuration bits, and commit them to the device. You obtain one from the
/// `Expander::configure()`, chain method calls on it to make configuration changes, and then end
/// the chain with `commit()` to transmit them to the MAX7301.
///
/// ```
/// # use max7301::interface::noop::NoopInterface;
/// # use max7301::expander::Expander;
/// # use max7301::config::PortMode;
/// # let ei = NoopInterface;
/// let mut expander = Expander::new(ei);
/// expander
///     .configure()
///     .ports(4..=7, PortMode::Output)
///     .shutdown(false)
///     .commit()
///     .unwrap();
/// ```
#[must_use = "Configuration changes are not applied unless committed"]
pub struct Configurator<'e, EI: ExpanderInterface + Send> {
    expander: &'e mut Expander<EI>,
    expander_config_dirty: bool,
    banks: [BankConfig; 7],
}

impl<'e, EI: ExpanderInterface + Send> Configurator<'e, EI> {
    pub(crate) fn new(expander: &'e mut Expander<EI>) -> Self {
        Self {
            expander,
            expander_config_dirty: false,
            banks: [BankConfig(0); 7],
        }
    }

    fn set_port(&mut self, port: u8, mode: PortMode) {
        let (bank, offset) = port_bank_and_offset(port);
        self.banks[bank as usize].set_port(offset, mode);
    }

    /// Set the port mode of a single GPIO pin on the MAX7301 to `mode`. `port` is the port number
    /// as specified in the device datasheet, in the range `4..=31`.
    pub fn port(mut self, port: u8, mode: PortMode) -> Self {
        self.set_port(port, mode);
        self
    }

    /// Set the port mode of a sequence of GPIO pins to the given `PortMode`. `ports` must yield
    /// values corresponding to port numbers as specified in the device datasheet, in the range
    /// `4..=31`. All of the ports will be set to mode `mode`.
    pub fn ports<I>(mut self, ports: I, mode: PortMode) -> Self
    where
        I: IntoIterator<Item = u8>,
    {
        for port in ports {
            self.set_port(port, mode);
        }
        self
    }

    /// Set the MAX7301's shutdown bit. When `false` the device will operate normally; when `true`
    /// the device enters shutdown mode. In shutdown mode all ports are overridden to input mode
    /// and pull-up current sources are disabled, but all registers retain their values and may be
    /// read and written normally. See the datasheet for more details.
    pub fn shutdown(mut self, enable: bool) -> Self {
        self.expander.config.shutdown = enable;
        self.expander_config_dirty = true;
        self
    }

    /// Set the MAX7301's transition detection feature control bit. When `false` the feature is
    /// disabled; when `true` ports 24 through 31 will be monitored for changes, setting an
    /// interrupt pin when they are detected. See datasheet for details. Interrupts generated from
    /// this hardware feature are not managed by this driver.
    pub fn detect_transitions(mut self, enable: bool) -> Self {
        self.expander.config.transition_detect = enable;
        self.expander_config_dirty = true;
        self
    }

    /// Commit the configuration changes to the MAX7301. The configurator will attempt to update
    /// the device's configuration registers while minimizing bus traffic (avoiding
    /// read-modify-writes when possible, not setting registers that were not changed).
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
        assert_eq!(u8::from(expander_config), 0b00000000);
    }

    #[test]
    fn expander_config_disable_shutdown() {
        let mut expander_config = ExpanderConfig::default();
        expander_config.shutdown = false;
        assert_eq!(u8::from(expander_config), 0b00000001);
    }

    #[test]
    fn expander_config_enable_transition_detect() {
        let mut expander_config = ExpanderConfig::default();
        expander_config.transition_detect = true;
        assert_eq!(u8::from(expander_config), 0b10000000);
    }
}
