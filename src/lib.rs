//! Driver library for the Maxim MAX7301 serial I/O expander.
//!
//! This driver is intended to work on embedded platforms using any implementation of the
//! `embedded-hal` trait library.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate core;

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
