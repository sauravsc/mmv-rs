# mmv-rs

[![CI](https://github.com/sauravsc/mmv-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/sauravsc/mmv-rs/actions/workflows/ci.yml)
![MSRV 1.85](https://img.shields.io/badge/MSRV-1.85-blue)
![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

A proof-of-concept, pure Rust implementation for writing
[Performance Co-Pilot](https://pcp.io) (PCP) Memory Mapped Value (MMV)
version 1 files.

> **Status: proof of concept.** I wrote this in 2017, during the community
> bonding period of my [GSoC project](https://medium.com/@saurvs/gsoc-2017-introduction-834825fb2aee),
> to compare a pure Rust approach with a Rust-to-C FFI approach
> ([pcp_mmv-rs](https://github.com/sauravsc/pcp_mmv-rs)) for writing MMV files.
> The project went on to take the pure Rust route, which became
> [**hornet**](https://github.com/performancecopilot/hornet)
> ([crates.io](https://crates.io/crates/hornet)). **For production use, use
> hornet**, which also supports instance domains, string metrics and
> histograms. This crate is kept building and tested, but is not
> published to crates.io (where the `mmv` name belongs to an unrelated
> crate).

It supports 64-bit integer and floating point metrics without instance
domains, and needs no C libraries. CI tests it on Linux (x86_64, aarch64) and
macOS, and checks its output with PCP's `mmvdump`.

## Usage

Requires Rust 1.85 or newer.

```toml
[dependencies]
mmv = { git = "https://github.com/sauravsc/mmv-rs" }
```

```rust
use mmv::{Metric, MetricSem, MetricType, Mmv, MmvFlags, Units};

let mut requests = Metric::new(
    "requests", 1, MetricSem::Counter, 0, Units::new(0, 0, 1, 0, 0, 0),
    MetricType::I64(0), "Requests", "Number of requests served",
);
// PCP's MMV agent reads files in $PCP_TMP_DIR/mmv.
Mmv::new("/var/lib/pcp/tmp/mmv/myapp", MmvFlags::empty(), 1).map(&mut [&mut requests])?;
requests.set_val(MetricType::I64(1));
```

## Example

`monte_pi` estimates Pi with Monte Carlo trials, one every 100ms, and
publishes the number of trials and the current estimate:

```sh
cargo run --example monte_pi -- /var/lib/pcp/tmp/mmv/monte_pi
```

With PCP's MMV agent installed (`cd /var/lib/pcp/pmdas/mmv && sudo ./Install`),
`pminfo -f mmv.monte_pi` shows the metrics while it runs. PCP's `mmvdump`
prints the contents of the file.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
