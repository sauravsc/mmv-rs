# Changelog

## 0.2.0 - 2026-11-27

Refresh of the 2017 proof of concept so that it builds and runs with current
Rust. For production use, see
[hornet](https://github.com/performancecopilot/hornet).

### Changed (breaking)

- Rust 2024 edition; the minimum supported Rust version is 1.85.
- `MMV` is renamed to `Mmv` and `MMVFlags` to `MmvFlags`.
- `Mmv::map` returns `io::Result<()>` instead of panicking, and rejects
  empty metric lists, duplicate metric names or items, and cluster IDs of
  4096 or more.
- `Metric::new` takes a `Units` value, with a `Units::new` constructor like
  PCP's `MMV_UNITS`, instead of a packed `u32`.
- `Mmv::new` takes any `AsRef<Path>`.
- Files are written in native byte order, as PCP expects, instead of always
  little-endian.
- Dependencies: `memmap` 0.5 (unmaintained) is replaced by `memmap2` 0.9 and
  `bitflags` is updated to 2. `byteorder`, `time` and `nix` are replaced by
  the standard library, and `rand` is now only a dev-dependency.

### Fixed

- `Mmv::map` creates the file if it does not exist, and replaces an existing
  file instead of overwriting it in place, so stale bytes at the end of a
  larger old file no longer remain and PCP's MMV agent reloads it.
- Values are written with volatile 8-byte stores through a shared mapping
  that each metric keeps alive, instead of through split mapping views.
- `monte_pi` used item 1 for both of its metrics, and takes an optional
  number of trials.

### Added

- Tests that check the file layout against PCP's `mmv_dev.h`, and GitHub
  Actions CI on Linux (x86_64, aarch64) and macOS (stable and 1.85) that
  also checks the example's output with `mmvdump`.
- License: MIT OR Apache-2.0.

## 0.1.0 - 2017-05-30

- Initial proof of concept.
