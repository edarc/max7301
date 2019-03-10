//! Driver library for the Maxim MAX7301 serial I/O expander.
//!
//! The MAX7301 is a device that provides either 20 or 28 GPIO pins, which are
//! software-configurable as push-pull output, floating input, or input with weak pull-up. The
//! state of each pin can be read and written through an SPI serial bus.
//!
//! This driver is intended to work on embedded platforms using any implementation of the
//! `embedded-hal` trait library. It communicates with the expander via any SPI and GPIO device
//! implementing the respective traits, and permits creation of new GPIO devices corresponding to
//! the I/O pins on the device, which themselves implement the HAL traits.
//!
//! # Construction
//!
//! To set up the driver:
//!
//! - Use your platform's `embedded-hal` implementation to obtain the necessary I/Os where your
//!   MAX7301 is connected. For the SPI version (currently the only supported version), you will
//!   need an SPI master device, and one GPIO push-pull output pin device for chip select.
//! - Construct an [`ExpanderInterface`] — the [`SpiInterface`] for MAX7301 — which will take
//!   ownership of the I/O devices you just obtained.
//! - Construct an [`Expander`], which will take ownership of the `ExpanderInterface`, and which will
//!   provide a builder API to configure the device.
//!
//! ```ignore
//! let spi = /* construct something implementing embedded_hal::spi::blocking::{Write,Transfer} */
//! let cs = /* construct something implementing embedded_hal::digital::OutputPin */
//!
//! let ei = max7301::SpiInterface::new(spi, cs);
//! let mut expander = max7301::Expander::new(ei);
//! ```
//!
//! # Device configuration
//!
//! *See [`Expander::configure`] and [`expander::Configurator`].*
//!
//! The `configure` method will produce a builder that you can use to set up the chip's
//! configuration registers:
//!
//! ```
//! # fn main() -> Result<(), ()> {
//! # let ei = max7301::interface::noop::NoopInterface;
//! # let mut expander = max7301::Expander::new(ei);
//! expander
//!     .configure()
//!     .ports(4..=31, max7301::PortMode::InputPullup)
//!     .port(7, max7301::PortMode::Output)
//!     .shutdown(false)
//!     .commit()?;
//! # Ok(())
//! # }
//! ```
//!
//! # Raw mode
//!
//! *See [`Expander`].*
//!
//! Now with a configured device, you may use it in raw mode, accessing the device's ports
//! directly:
//!
//! ```
//! # fn main() -> Result<(), ()> {
//! # let ei = max7301::interface::noop::NoopInterface;
//! # let mut expander = max7301::Expander::new(ei);
//! let four_thru_twelve: u8 = expander.read_ports(4)?;
//! expander.write_port(7, false)?;
//! # Ok(())
//! # }
//! ```
//!
//! # HAL modes
//!
//! To compose the driver with other `embedded-hal` drivers that are compatible with
//! `embedded_hal::digital::{InputPin,OutputPin}`, you can convert the `Expander` into an I/O
//! adapter that will produce ownable `PortPin` instances for each GPIO port on the expander.
//! `PortPin` implments the `InputPin` and `OutputPin` traits from `embedded-hal`, allowing
//! composition with other drivers that need GPIO pins.
//!
//! You can choose one of two interfaces to construct `PortPin`s:
//!
//! - The **immediate mode** interface, where calling the GPIO trait methods on any `PortPin`
//!   immediately generates a bus transaction to the expander in order to perform the operation on
//!   that pin; or
//! - The **transactional** interface, which allows you to control the bus traffic separately from
//!   the activity on the `PortPin`s, enabling batching of reads and writes to amortize and/or
//!   reduce bus traffic and latency.
//!
//! ## Immediate mode
//!
//! *See [`Expander::into_immediate`] and [`ImmediateIO`].*
//!
//! ```
//! # struct MyTrafficLight<P>(core::marker::PhantomData<P>);
//! # impl<P> MyTrafficLight<P> where P: embedded_hal::digital::OutputPin {
//! #   fn new(r: P, y: P, g: P) -> Self { Self(core::marker::PhantomData) }
//! #   fn change_to_red(&mut self) {}
//! # }
//! # fn main() -> Result<(), ()> {
//! # let ei = max7301::interface::noop::NoopInterface;
//! # let mut expander = max7301::Expander::new(ei);
//! expander.configure().ports(4..=6, max7301::PortMode::Output).commit()?;
//! let imm_io = expander.into_immediate::<max7301::DefaultMutex<_>>();
//!
//! let red_lamp = imm_io.port_pin(4);
//! let yellow_lamp = imm_io.port_pin(5);
//! let green_lamp = imm_io.port_pin(6);
//! let mut traffic_light = MyTrafficLight::new(red_lamp, yellow_lamp, green_lamp);
//!
//! traffic_light.change_to_red();
//! # Ok(())
//! # }
//! ```
//!
//! In this example, each time `MyTrafficLight` interacts with an `OutputPin` trait method, the
//! driver will immediately trigger a bus transaction to set the appropriate level on the
//! expander's corresponding output pin. Likewise, if an `InputPin` trait method is called, the
//! driver will perform a bus transaction to read the current state from the expander pin.
//!
//! ## Transactional mode
//!
//! *See [`Expander::into_transactional`] and [`TransactionalIO`].*
//!
//! ```
//! # struct MyFancyTrafficLight<P>(core::marker::PhantomData<P>);
//! # impl<P> MyFancyTrafficLight<P> where P: embedded_hal::digital::OutputPin {
//! #   fn new(r: P, y: P, g: P, s: P) -> Self { Self(core::marker::PhantomData) }
//! #   fn change_if_tripped(&mut self) {}
//! # }
//! # fn main() -> Result<(), ()> {
//! # let ei = max7301::interface::noop::NoopInterface;
//! # let mut expander = max7301::Expander::new(ei);
//! expander
//!     .configure()
//!     .ports(4..=6, max7301::PortMode::Output)
//!     .port(7, max7301::PortMode::InputFloating)
//!     .commit()?;
//! let txn_io = expander.into_transactional::<max7301::DefaultMutex<_>>();
//!
//! let red_lamp = txn_io.port_pin(4);
//! let yellow_lamp = txn_io.port_pin(5);
//! let green_lamp = txn_io.port_pin(6);
//! let sensor_tripped = txn_io.port_pin(7);
//! let mut traffic_light =
//!     MyFancyTrafficLight::new(red_lamp, yellow_lamp, green_lamp, sensor_tripped);
//!
//! txn_io.refresh()?;
//! traffic_light.change_if_tripped();
//! txn_io.write_back(max7301::Strategy::Exact)?;
//! # Ok(())
//! # }
//! ```
//!
//! In this example, the transactional API adds two extra methods on the I/O adapter: `refresh()`
//! and `write_back()`.
//!
//! - The `refresh` method instructs the driver to perform bus operations to load the current state
//!   of every port for which a `PortPin` exists into its internal write-back cache.
//! - As `MyTrafficLight` interacts with the `PortPin` trait methods, they read and mutate the
//!   state of the ports in the write-back cache, without interacting with the hardware.
//! - Finally, `write_back` is called to instruct the driver to perform bus operations to write any
//!   modified port states back to the expander. The single argument selects the strategy that will
//!   be used to batch the write operations into fewer write cycles on the bus (see
//!   [`expander::transactional::Strategy`] for a description).
//!
//! ## Choosing a mode
//!
//! - Transactional mode is excellent for applications which use the MAX7301 to obtain a large
//!   number of *independent* GPIOs, for example HMI applications like keypads, switches, encoders,
//!   sensors, LEDs, and indicators. In such a case, the states of the GPIOs are often read or
//!   updated in a procedure that can be bracketed by `refresh` and `write_back`, since the order
//!   in which the states are read from or written out to the hardware is not important.
//! - Transactional mode is not appropriate, and immediate mode should be used instead, for drivers
//!   or applications that use the generated GPIOs in a "bit-banged" manner, where pins must
//!   transition states with particular timings or orderings with respect to each other. In
//!   transactional mode, the write-back cache delays the hardware transitions until the next
//!   `write_back` call, erasing any sequencing or timing and breaking these use cases.
//!
//! ## Mutual exclusion
//!
//! The HAL adapters require you to provide a mutual exclusion primitive to arbitrate access to the
//! hardware from multiple `PortPin`s. For now, the adapters are parameterized over a type
//! implementing the `IOMutex` trait, which is a concept borrowed from
//! [`shared-bus`](http://docs.rs/shared-bus), and which will hopefully be superseded by [a
//! standard `embedded-hal` trait](https://github.com/rust-embedded/embedded-hal/issues/119).
//!
//! In a `std` environment you may enable the `std` Cargo feature, and `mutex::DefaultMutex<T>`
//! will be a type alias to `std::sync::Mutex<T>` with a provided impl of `IOMutex`. Similarly, for
//! Cortex-M environments using the `cortex-m` crate, enabling the `cortexm` Cargo feature will
//! alias `mutex::DefaultMutex<T>` to `cortex_m::interrupt::Mutex<core::cell::RefCell<T>>` with a
//! provided `IOMutex` impl. This arrangement should allow you to just specify `DefaultMutex` as in
//! the examples, and have the correct thing happen based on the build environment.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate core;
#[cfg(test)]
extern crate proptest;

extern crate embedded_hal as hal;

pub mod config;
pub mod expander;
pub mod interface;
pub mod mutex;
pub mod registers;

pub use config::PortMode;
pub use expander::immediate::ImmediateIO;
pub use expander::pin::{ExpanderIO, PortPin};
pub use expander::transactional::{Strategy, TransactionalIO};
pub use expander::Expander;
pub use interface::spi::SpiInterface;
pub use interface::ExpanderInterface;
pub use mutex::{DefaultMutex, IOMutex};
