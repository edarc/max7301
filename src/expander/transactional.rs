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
    use interface::test_spy::{TestRegister as TR, TestSpyInterface};
    use mutex::DefaultMutex;

    #[test]
    fn single_pin_read() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_twelve = io.port_pin(12);

        ei.set(0x4C, TR::ResetValue(0x00));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_twelve.is_high(), false);

        ei.set(0x4C, TR::ResetValue(0x01));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_twelve.is_high(), true);
    }

    #[test]
    #[should_panic]
    fn read_unrefreshed_panics() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_twelve = io.port_pin(12);

        pin_twelve.is_high();
    }

    #[test]
    fn multi_pin_read_single_register() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_twelve = io.port_pin(12);
        let pin_fifteen = io.port_pin(15);

        ei.set(0x4C, TR::ResetValue(0b00001001));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_twelve.is_high(), true);
        assert_eq!(pin_fifteen.is_high(), true);
        assert_eq!(ei.reads(), vec![0x4C]);
    }

    #[test]
    fn multi_pin_read_adjoining_registers() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_eleven = io.port_pin(11);
        let pin_nineteen = io.port_pin(19);

        ei.set(0x4B, TR::ResetValue(0b00000001));
        ei.set(0x53, TR::ResetValue(0b00000001));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_eleven.is_high(), true);
        assert_eq!(pin_nineteen.is_high(), true);
        assert_eq!(ei.reads(), vec![0x4B, 0x53]);
    }

    #[test]
    fn multi_pin_read_disjoint_registers() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_eleven = io.port_pin(11);
        let pin_twentytwo = io.port_pin(22);

        ei.set(0x4B, TR::ResetValue(0b00000001));
        ei.set(0x56, TR::ResetValue(0b00000001));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_eleven.is_high(), true);
        assert_eq!(pin_twentytwo.is_high(), true);
        assert_eq!(ei.reads(), vec![0x4B, 0x56]);
    }

    #[test]
    fn multi_pin_read_edges() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_four = io.port_pin(4);
        let pin_thirtyone = io.port_pin(31);

        ei.set(0x44, TR::ResetValue(0b00000001));
        ei.set(0x5F, TR::ResetValue(0b00000001));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_four.is_high(), true);
        assert_eq!(pin_thirtyone.is_high(), true);
        assert_eq!(ei.reads(), vec![0x44, 0x5F]);
    }

    #[test]
    fn multi_pin_read_end_at_upper_edge() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let pin_twentyfour = io.port_pin(24);
        let pin_thirtyone = io.port_pin(31);

        ei.set(0x58, TR::ResetValue(0b10000001));
        assert!(io.refresh().is_ok());
        assert_eq!(pin_twentyfour.is_high(), true);
        assert_eq!(pin_thirtyone.is_high(), true);
        assert_eq!(ei.reads(), vec![0x58]);
    }

    #[test]
    fn single_pin_write_exact() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);

        pin_twelve.set_high();
        assert_eq!(ei.get(0x2C), TR::ResetValue(0x00));
        assert!(io.write_back(Strategy::Exact).is_ok());
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0x01));
    }

    #[test]
    fn multi_pin_write_exact_single_port_registers() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);
        let mut pin_fifteen = io.port_pin(15);

        pin_twelve.set_high();
        pin_fifteen.set_high();
        assert_eq!(ei.get(0x2C), TR::ResetValue(0x00));
        assert_eq!(ei.get(0x2F), TR::ResetValue(0x00));
        assert!(io.write_back(Strategy::Exact).is_ok());
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0b00000001));
        assert_eq!(ei.get(0x2F), TR::WrittenValue(0b00000001));
    }

    #[test]
    fn multi_pin_write_exact_multi_port_register() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let mut eight_pins = (11..=18)
            .into_iter()
            .map(|p| io.port_pin(p))
            .collect::<Vec<_>>();

        eight_pins.iter_mut().for_each(|p| p.set_high());
        assert_eq!(ei.get(0x4B), TR::ResetValue(0b00000000));
        assert!(io.write_back(Strategy::Exact).is_ok());
        assert_eq!(ei.get(0x4B), TR::WrittenValue(0b11111111));
    }

    #[test]
    fn multi_pin_write_clean_after_refresh_single_range_register() {
        let mut ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);
        let mut pin_fifteen = io.port_pin(15);
        let _pin_seventeen = io.port_pin(17);

        ei.set(0x4C, TR::ResetValue(0b00100000));
        assert!(io.refresh().is_ok());
        pin_twelve.set_high();
        pin_fifteen.set_high();
        assert_eq!(ei.get(0x4C), TR::ResetValue(0b00100000));
        assert!(io.write_back(Strategy::StompClean).is_ok());
        assert_eq!(ei.get(0x4C), TR::WrittenValue(0b00101001));
    }

    #[test]
    fn multi_pin_write_clean_no_refresh_single_port_registers() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);
        let mut pin_fifteen = io.port_pin(15);

        pin_twelve.set_high();
        pin_fifteen.set_high();
        assert_eq!(ei.get(0x2C), TR::ResetValue(0x00));
        assert_eq!(ei.get(0x2F), TR::ResetValue(0x00));
        assert!(io.write_back(Strategy::StompClean).is_ok());
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0b00000001));
        assert_eq!(ei.get(0x2F), TR::WrittenValue(0b00000001));
    }

    #[test]
    fn multi_pin_write_any_no_refresh_single_range_register() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();
        let mut pin_twelve = io.port_pin(12);
        let mut pin_fifteen = io.port_pin(15);

        pin_twelve.set_high();
        pin_fifteen.set_high();
        assert_eq!(ei.get(0x4C), TR::ResetValue(0x00));
        assert!(io.write_back(Strategy::StompAny).is_ok());
        assert_eq!(ei.get(0x4C), TR::WrittenValue(0b00001001));
    }

    #[test]
    fn multi_pin_write_clean_dont_stomp_unrefreshed() {
        let ei = TestSpyInterface::new();
        let io = Expander::new(ei.split()).into_transactional::<DefaultMutex<_>>();

        let _pin_fourteen = io.port_pin(14);
        assert!(io.refresh().is_ok());
        // Ports 14-21 are clean.

        let mut pin_twelve = io.port_pin(12);
        pin_twelve.set_high();
        // Port twelve is dirty, but 13 is not fresh. Do not stomp it.

        assert!(io.write_back(Strategy::StompClean).is_ok());
        assert_eq!(ei.get(0x2C), TR::WrittenValue(0b00000001));
    }
}
