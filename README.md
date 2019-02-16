# MAX7301 serial I/O expander driver

Pure Rust driver for the Maxim MAX7301 serial I/O expander chip, for use with
[embedded-hal](https://crates.io/crates/embedded-hal).

## Description

This driver is intended to work on embedded platforms using the `embedded-hal`
trait library. It is `no_std`, contains no added `unsafe`, and does not require
an allocator. The initial release supports the 4-wire SPI interface.

## License

Licensed under either of

- Apache License, Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0)
- MIT license (http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
