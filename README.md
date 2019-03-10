# MAX7301 serial I/O expander driver

Pure Rust driver for the Maxim MAX7301 serial I/O expander chip, for use with
[embedded-hal](https://crates.io/crates/embedded-hal).

## Description

This driver is intended to work on embedded platforms using the `embedded-hal`
trait library. It is `no_std` and does not require an allocator. The initial
release supports the MAX7301 which uses an SPI interface. I would like (and
have left interfaces) to extend it to support the MAX7300, which is an
equivalent device with an I2C interface instead.

The driver allows three different styles of using the device:

- A "raw" interface, which exposes minimally-abstracted methods that directly
  map to the operations the device implements, but which do not map onto
  `embedded-hal` traits,
- An *immediate mode* interface, which allows creation of individual, ownable
  `PortPin` instances that implement the `InputPin` and `OutputPin` traits,
  where calling any methods from those traits immediately initiates a bus
  transaction to perform the operation, and
- A *transactional* interface, which similarly allows creation of `PortPin`
  instances, but where the HAL trait methods do not generate bus traffic
  directly, but instead interact with a write-back cache inside the driver.
  There are additional methods that control when the write-back cache is
  refreshed or written back, and these methods permit the driver to exploit the
  device's multi-port registers to reduce the amount of bus traffic and latency
  substantially in situations where it makes sense.

### Missing features:

- I2C interface for MAX7300 variant.
- Helper methods for using the device's hardware transition detection and
  interrupt generator. It can be enabled and disabled, but doing so is a bit
  pointless as there is no method to alter the port mask, and no method to
  clear the interrupt it once it is triggered.

## License

Licensed under either of

- Apache License, Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0)
- MIT license (http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
