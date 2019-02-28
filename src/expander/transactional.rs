//! Transactional I/O adapter.

use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};

use expander::pin::{ExpanderIO, PortPin};
use expander::Expander;
use interface::ExpanderInterface;
use mutex::IOMutex;
use registers::valid_port;

/// Control how `TransactionalIO::write_back` will batch writes to modified pins.
pub enum Strategy {
    /// This strategy will issue writes such that only the ports that have had output values
    /// explicitly set through the `OutputPin` impl will be altered.
    ///
    /// This is the safest but least efficient write back strategy.
    Exact,

    /// This strategy will relax the write back batching so that it may overwrite any port that had
    /// its state read and cached in the most recent `refresh` call.
    ///
    /// This means that some port registers may be "stomped" by writing values that match the
    /// values they had when `refresh` was called. This is true regardless of whether the port is
    /// configured as an input or output pin.
    StompClean,

    /// This strategy will further relax the write-back batching so that may potentially overwrite
    /// *any* port, even if the previous value was not read in a `refresh`.
    ///
    /// Any ports that were not read in a `refresh` will be overwritten with an undefined value.
    /// This strategy makes most efficient use of the bus when most pins are output pins, but is
    /// only usable if you call `port_pin` for every port you care about, and *either*
    ///
    /// * Explicitly set every pin whose value you care about before each `write_back` call, or
    /// * Call `refresh` first before setting any pins.
    StompAny,
}

/// This I/O adapter captures the `Expander` and provides a factory for generating GPIO pins that
/// implement `InputPin` and `OutputPin` traits backed by a transactional write-back cache.
///
/// Each such pin will read its input state from a cached batch read at the beginning of a
/// transaction, and will write its input state into a write-back buffer that is committed with a
/// batch write at the end of the transaction. This reduces bus traffic due to the MAX7301's
/// support for reading or writing 8 consecutive ports in a single operation.
pub struct TransactionalIO<M, EI>
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface,
{
    expander: M,
    issued: AtomicUsize,
    cache: AtomicUsize,
    dirty: AtomicUsize,
    fresh: AtomicUsize,
    _ei: PhantomData<EI>,
}

// Unsafety: This is only needed because the presence of PhantomData<EI> causes the struct to no
// longer be Sync, because EI is often not Sync since it owns a global resource (e.g. SPI device).
// However, the EI is actually owned by the Expander which is in the mutex which normally
// re-instates Sync-ness. PhantomData is there to shut up the unused type parameter error.
unsafe impl<M, EI> Sync for TransactionalIO<M, EI>
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface,
{
}

impl<M, EI> TransactionalIO<M, EI>
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface,
{
    pub(crate) fn new(expander: Expander<EI>) -> Self {
        TransactionalIO {
            expander: M::new(expander),
            issued: AtomicUsize::default(),
            cache: AtomicUsize::default(),
            dirty: AtomicUsize::default(),
            fresh: AtomicUsize::default(),
            _ei: PhantomData,
        }
    }

    /// Create a `PortPin` corresponding to one of the ports on the MAX7301. The returned `PortPin`
    /// implements `InputPin` and `OutputPin`, and using any of the methods from these traits on
    /// the returned `PortPin` will read or write the value of the I/O port from a local write-back
    /// cache. Refreshing or writing back the cache is controlled by `refresh` and `write_back`.
    pub fn port_pin<'io>(&'io self, port: u8) -> PortPin<'io, Self> {
        self.issued
            .fetch_or(1 << valid_port(port), Ordering::Relaxed);
        PortPin::new(self, port)
    }

    /// Refresh the local cache by reading the port values from any outstanding `PortPin`s issued
    /// from this adapter, updating the values read through their `InputPin` impls. This is done
    /// using batch registers of MAX7301 to reduce bus traffic. All pending `OutputPin` operations
    /// are discarded.
    pub fn refresh(&self) -> Result<(), ()> {
        self.dirty.store(0, Ordering::Release);
        let mut load_buffer = 0usize;
        let mut fresh_buffer = 0usize;
        let mut start_port = 4;
        let mut ports_to_read = self.issued.load(Ordering::Relaxed) >> start_port;
        while ports_to_read != 0 {
            let skip = ports_to_read.trailing_zeros();
            ports_to_read >>= skip;
            start_port += skip;
            let port_values = self.expander.lock(|ex| ex.read_ports(start_port as u8))?;
            load_buffer |= (port_values as usize) << start_port;
            fresh_buffer |= 0xFFusize << start_port;
            ports_to_read &= !0xFFusize;
        }
        self.cache.store(load_buffer, Ordering::Relaxed);
        self.fresh.store(fresh_buffer, Ordering::Relaxed);
        Ok(())
    }

    /// Write back any pending `OutputPin` operations to the MAX7301. The strategy used to do this
    /// is controlled by `strategy` (see [`Strategy`] docs for a description of the available
    /// strategies).
    pub fn write_back(&self, strategy: Strategy) -> Result<(), ()> {
        let mut start_port = 0;
        let mut ports_to_write = self.dirty.load(Ordering::Acquire);
        let mut ok_to_write = match strategy {
            Strategy::Exact => ports_to_write,
            Strategy::StompClean => self.fresh.load(Ordering::Acquire),
            Strategy::StompAny => 0xFFFFFFFC,
        };
        let cache = self.cache.load(Ordering::Acquire);
        while ports_to_write != 0 {
            let skip = ports_to_write.trailing_zeros();
            ports_to_write >>= skip;
            ok_to_write >>= skip;
            start_port += skip;
            if ok_to_write & 0xFF == 0xFF {
                let port_values = (cache >> start_port) as u8;
                self.expander
                    .lock(|ex| ex.write_ports(start_port as u8, port_values))?;
                ports_to_write &= !0xFFusize;
            } else {
                let port_value = cache & (1 << start_port) != 0;
                self.expander
                    .lock(|ex| ex.write_port(start_port as u8, port_value))?;
                ports_to_write &= !0x01usize;
            }
        }
        self.dirty.store(0, Ordering::Release);
        Ok(())
    }
}

impl<M, EI> ExpanderIO for TransactionalIO<M, EI>
where
    M: IOMutex<Expander<EI>>,
    EI: ExpanderInterface,
{
    fn write_port(&self, port: u8, bit: bool) {
        let or_bit = 1 << port;
        if bit {
            self.cache.fetch_or(or_bit, Ordering::Release);
        } else {
            self.cache.fetch_and(!or_bit, Ordering::Release);
        }
        self.dirty.fetch_or(or_bit, Ordering::Relaxed);
        self.fresh.fetch_or(or_bit, Ordering::Relaxed);
    }
    fn read_port(&self, port: u8) -> bool {
        if self.fresh.load(Ordering::Relaxed) & (1 << port) == 0 {
            panic!("Read of un-refreshed port;}")
        }
        self.cache.load(Ordering::Relaxed) & (1 << port) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::Strategy;
    use expander::Expander;
    use hal::digital::{InputPin, OutputPin};
    use interface::test_spy::{SemanticTestSpyInterface, TestPort};
    use mutex::DefaultMutex;
    use proptest::collection::vec;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        #[test]
        fn prop_read_unrefreshed_panics(
            reset in vec(any::<bool>(), 32 - 4),
            pin in 4..=31u8
        ) {
            assert!(std::panic::catch_unwind(|| {
                let ei = SemanticTestSpyInterface::new(reset);
                let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
                let any_pin = io.port_pin(pin);

                any_pin.is_high();
            })
            .is_err());
        }

        #[test]
        fn prop_read_refreshed_ok(
            reset in vec(any::<bool>(), 32 - 4),
            pins in vec(4..=31u8, 0..=28)
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let some_pins = pins
                .iter()
                .cloned()
                .map(|p| io.port_pin(p))
                .collect::<Vec<_>>();

            assert!(io.refresh().is_ok());
            for (idx, pin_nr) in pins.iter().enumerate() {
                assert_eq!(some_pins[idx].is_high(), reset[*pin_nr as usize - 4]);
            }
        }

        // Precisely the pins that were modified will be written back using blind writes. No other
        // ports should be written.
        #[test]
        fn prop_write_exact_no_refresh(
            reset in vec(any::<bool>(), 32 - 4),
            pins_and_bits in vec((4..=31u8, any::<bool>()), 0..=28),
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let mut some_pins = pins_and_bits
                .iter()
                .cloned()
                .map(|(p, b)| (p, io.port_pin(p), b))
                .collect::<Vec<_>>();
            let mut expect = (4..=31)
                .into_iter()
                .map(|p| TestPort::Reset(reset[p as usize - 4]))
                .collect::<Vec<_>>();

            for (port, pin, bit) in some_pins.iter_mut() {
                if *bit {
                    pin.set_high()
                } else {
                    pin.set_low()
                }
                expect[*port as usize - 4] = TestPort::BlindWrite(*bit);
            }
            assert!(io.write_back(Strategy::Exact).is_ok());
            assert_eq!(expect, ei.peek_all());
        }

        // Precisely the pins that were modified will be written back using read-writes. No other
        // ports should be written.
        #[test]
        fn prop_write_exact_with_refresh(
            reset in vec(any::<bool>(), 32 - 4),
            pins_and_bits in vec((4..=31u8, any::<bool>()), 0..=28),
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let mut some_pins = pins_and_bits
                .iter()
                .cloned()
                .map(|(p, b)| (p, io.port_pin(p), b))
                .collect::<Vec<_>>();
            let mut expect = (4..=31)
                .into_iter()
                .map(|p| TestPort::Reset(reset[p as usize - 4]))
                .collect::<Vec<_>>();

            assert!(io.refresh().is_ok());
            for (port, pin, bit) in some_pins.iter_mut() {
                if *bit {
                    pin.set_high()
                } else {
                    pin.set_low()
                }
                expect[*port as usize - 4] = TestPort::ReadWrite(*bit);
            }
            assert!(io.write_back(Strategy::Exact).is_ok());
            assert_eq!(expect, ei.peek_all());
        }

        // StompClean strategy should behave identically to Exact strategy if no refresh has
        // occurred, using blind writes to write exactly the ports whose pin was modified.
        #[test]
        fn prop_write_clean_no_refresh(
            reset in vec(any::<bool>(), 32 - 4),
            pins_and_bits in vec((4..=31u8, any::<bool>()), 0..=28),
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let mut some_pins = pins_and_bits
                .iter()
                .cloned()
                .map(|(p, b)| (p, io.port_pin(p), b))
                .collect::<Vec<_>>();
            let mut expect = (4..=31)
                .into_iter()
                .map(|p| TestPort::Reset(reset[p as usize - 4]))
                .collect::<Vec<_>>();

            for (port, pin, bit) in some_pins.iter_mut() {
                if *bit {
                    pin.set_high()
                } else {
                    pin.set_low()
                }
                expect[*port as usize - 4] = TestPort::BlindWrite(*bit);
            }
            assert!(io.write_back(Strategy::StompClean).is_ok());
            assert_eq!(expect, ei.peek_all());
        }

        // StompClean strategy will preserve all reset values that do not correspond to pins that
        // were written to. Ports for written pins should contain the new bit. No blind writes
        // should occur.
        #[test]
        fn prop_write_clean_with_refresh(
            reset in vec(any::<bool>(), 32 - 4),
            pins_and_bits in vec((4..=31u8, any::<bool>()), 0..=28),
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let mut some_pins = pins_and_bits
                .iter()
                .cloned()
                .map(|(p, b)| (p, io.port_pin(p), b))
                .collect::<Vec<_>>();
            let mut expect = reset.clone();

            assert!(io.refresh().is_ok());
            for (port, pin, bit) in some_pins.iter_mut() {
                if *bit {
                    pin.set_high()
                } else {
                    pin.set_low()
                }
                expect[*port as usize - 4] = *bit;
            }
            assert!(io.write_back(Strategy::StompClean).is_ok());
            assert_eq!(expect, ei.peek_bits());
            assert!(
                !ei.peek_all().iter().any(|p| match p {
                    TestPort::BlindWrite(_) => true,
                    _ => false,
                }),
                "{:?}",
                ei.peek_all()
            );
        }

        // StompAny strategy when no refresh is occurred may use blind writes on any ports. The
        // ports that were written will have the new bits, no other guarantees are given.
        #[test]
        fn prop_write_any_no_refresh(
            reset in vec(any::<bool>(), 32 - 4),
            pins_and_bits in vec((4..=31u8, any::<bool>()), 0..=28),
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let mut some_pins = pins_and_bits
                .iter()
                .cloned()
                .map(|(p, b)| (p, io.port_pin(p), b))
                .collect::<Vec<_>>();
            let mut expect = std::collections::BTreeMap::<u8, bool>::new();

            for (port, pin, bit) in some_pins.iter_mut() {
                if *bit {
                    pin.set_high()
                } else {
                    pin.set_low()
                }
                expect.insert(*port, *bit);
            }
            assert!(io.write_back(Strategy::StompAny).is_ok());
            for (port, bit) in expect.iter() {
                assert_eq!(
                    ei.peek_all()[*port as usize - 4],
                    TestPort::BlindWrite(*bit)
                );
            }
        }

        // StompAny strategy when a refresh has occurred may use blind writes on any ports *that
        // were not read during the refresh*. Ports that were written will have the new bits, and
        // no active pin's port will be blindly written. No guarantees are given about other ports.
        #[test]
        fn prop_write_any_with_refresh(
            reset in vec(any::<bool>(), 32 - 4),
            pins_and_bits in vec((4..=31u8, any::<bool>()), 0..=28),
        ) {
            let ei = SemanticTestSpyInterface::new(reset.clone());
            let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
            let mut some_pins = pins_and_bits
                .iter()
                .cloned()
                .map(|(p, b)| (p, io.port_pin(p), b))
                .collect::<Vec<_>>();
            let mut expect = std::collections::BTreeMap::<u8, bool>::new();

            assert!(io.refresh().is_ok());
            for (port, pin, bit) in some_pins.iter_mut() {
                if *bit {
                    pin.set_high()
                } else {
                    pin.set_low()
                }
                expect.insert(*port, *bit);
            }
            assert!(io.write_back(Strategy::StompAny).is_ok());
            for (port, bit) in expect.iter() {
                assert_eq!(
                    ei.peek_all()[*port as usize - 4],
                    TestPort::ReadWrite(*bit)
                );
            }
        }
    }
}
