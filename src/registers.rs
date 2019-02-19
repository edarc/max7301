//! The register addresses within the MAX7301.

use interface::RegisterAddress;

pub enum Register {
    /// The no-op register. Reading or writing this register has no effect.
    Noop,

    /// Global configuration.
    Configuration,

    /// Transition detect mask.
    TransitionDetectMask,

    /// Port bank configuration.
    BankConfig(u8),

    /// Single-port registers.
    SinglePort(u8),

    /// Port range registers.
    PortRange(u8),
}

pub(crate) fn valid_port(port: u8) -> u8 {
    match port {
        4..=31 => port,
        _ => panic!("MAX7301 does not have port {}", port),
    }
}

fn valid_bank(bank: u8) -> u8 {
    match bank {
        0..=6 => bank,
        _ => panic!("MAX7301 does not have bank {}", bank),
    }
}

impl From<Register> for RegisterAddress {
    fn from(reg: Register) -> RegisterAddress {
        use self::Register::*;
        match reg {
            Noop => RegisterAddress(0x00),
            Configuration => RegisterAddress(0x04),
            TransitionDetectMask => RegisterAddress(0x06),
            BankConfig(bank) => RegisterAddress(valid_bank(bank) + 0x09),
            SinglePort(port) => RegisterAddress(valid_port(port) + 0x20),
            PortRange(start_port) => RegisterAddress(valid_port(start_port) + 0x40),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface::RegisterAddress;

    #[test]
    fn bank_config_address_valid() {
        assert!(RegisterAddress::from(Register::BankConfig(0)) == RegisterAddress(0x09));
        assert!(RegisterAddress::from(Register::BankConfig(6)) == RegisterAddress(0x0F));
    }

    #[test]
    #[should_panic]
    fn bank_config_address_invalid() {
        RegisterAddress::from(Register::BankConfig(7));
    }

    #[test]
    fn single_port_address_valid() {
        assert!(RegisterAddress::from(Register::SinglePort(4)) == RegisterAddress(0x24));
        assert!(RegisterAddress::from(Register::SinglePort(31)) == RegisterAddress(0x3F));
    }

    #[test]
    #[should_panic]
    fn single_port_address_invalid() {
        RegisterAddress::from(Register::SinglePort(37));
    }

    #[test]
    fn port_range_address_valid() {
        assert!(RegisterAddress::from(Register::PortRange(4)) == RegisterAddress(0x44));
        assert!(RegisterAddress::from(Register::PortRange(31)) == RegisterAddress(0x5F));
    }

    #[test]
    #[should_panic]
    fn port_range_address_invalid() {
        RegisterAddress::from(Register::PortRange(37));
    }
}
