//! Checks the MMV v1 files written by this crate against the layout that
//! PCP's `mmv_dev.h` defines.

use mmv::{Metric, MetricSem, MetricType, Mmv, MmvFlags, Units};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process;

/// An MMV file path in a fresh temporary directory, removed on drop.
struct TempMmv(PathBuf);

impl TempMmv {
    fn new(name: &str) -> TempMmv {
        let dir = std::env::temp_dir().join(format!("mmv-test-{}-{name}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        TempMmv(dir.join(name))
    }

    fn read(&self) -> File {
        File(fs::read(&self.0).unwrap())
    }
}

impl Drop for TempMmv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.parent().unwrap());
    }
}

/// The bytes of an MMV file, with native-endian accessors.
struct File(Vec<u8>);

impl File {
    fn u32(&self, offset: u64) -> u32 {
        let offset = offset as usize;
        u32::from_ne_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    fn u64(&self, offset: u64) -> u64 {
        let offset = offset as usize;
        u64::from_ne_bytes(self.0[offset..offset + 8].try_into().unwrap())
    }

    fn str(&self, offset: u64, max_len: usize) -> &str {
        let bytes = &self.0[offset as usize..offset as usize + max_len];
        let len = bytes.iter().position(|&b| b == 0).expect("NUL-terminated");
        std::str::from_utf8(&bytes[..len]).unwrap()
    }

    /// Returns (type, count, offset) of TOC entry `i`.
    fn toc(&self, i: u64) -> (u32, u32, u64) {
        let toc = 40 + 16 * i;
        (self.u32(toc), self.u32(toc + 4), self.u64(toc + 8))
    }
}

fn counter(name: &str, item: u32) -> Metric {
    Metric::new(
        name,
        item,
        MetricSem::Counter,
        0,
        Units::new(0, 0, 1, 0, 0, 0),
        MetricType::I64(0),
        "short",
        "long",
    )
}

#[test]
fn writes_mmv_v1_layout() {
    let file = TempMmv::new("layout");
    let mut requests = Metric::new(
        "requests",
        7,
        MetricSem::Counter,
        0,
        Units::new(0, 0, 1, 0, 0, 0),
        MetricType::I64(-3),
        "Requests",
        "Number of requests",
    );
    let mut latency = Metric::new(
        "latency",
        8,
        MetricSem::Instant,
        0,
        Units::new(0, 1, 0, 0, 1, 0),
        MetricType::F64(0.5),
        "Latency",
        "",
    );
    Mmv::new(&file.0, MmvFlags::PROCESS | MmvFlags::NOPREFIX, 321)
        .map(&mut [&mut requests, &mut latency])
        .unwrap();
    let f = file.read();

    // Header.
    assert_eq!(&f.0[0..4], b"MMV\0");
    assert_eq!(f.u32(4), 1, "version");
    assert_ne!(f.u64(8), 0, "generation");
    assert_eq!(f.u64(8), f.u64(16), "generation numbers match");
    assert_eq!(f.u32(24), 3, "TOC count");
    assert_eq!(f.u32(28), 3, "flags");
    assert_eq!(f.u32(32), process::id(), "pid");
    assert_eq!(f.u32(36), 321, "cluster");

    // Table of contents.
    let (metric_type, n_metrics, metrics) = f.toc(0);
    let (value_type, n_values, values) = f.toc(1);
    let (string_type, n_strings, strings) = f.toc(2);
    assert_eq!((metric_type, n_metrics, metrics), (3, 2, 88));
    assert_eq!((value_type, n_values, values), (4, 2, 88 + 2 * 104));
    assert_eq!((string_type, n_strings), (5, 4));
    assert_eq!(strings, values + 2 * 32);
    assert_eq!(f.0.len() as u64, strings + 4 * 256);

    // Metrics: name, item, type, semantics, units, indom, help offsets.
    let expected = [
        (
            "requests",
            7,
            2,
            1,
            0x0010_0000,
            "Requests",
            "Number of requests",
        ),
        ("latency", 8, 5, 3, 0x0100_1000, "Latency", ""),
    ];
    for (i, (name, item, type_, sem, units, short, long)) in expected.into_iter().enumerate() {
        let m = metrics + 104 * i as u64;
        assert_eq!(f.str(m, 64), name);
        assert_eq!(f.u32(m + 64), item);
        assert_eq!(f.u32(m + 68), type_);
        assert_eq!(f.u32(m + 72), sem);
        assert_eq!(f.u32(m + 76), units);
        assert_eq!(f.u32(m + 80), 0, "indom");
        assert_eq!(f.str(f.u64(m + 88), 256), short);
        assert_eq!(f.str(f.u64(m + 96), 256), long);

        // Values point back at their metric and have no instance.
        let v = values + 32 * i as u64;
        assert_eq!(f.u64(v + 16), m);
        assert_eq!(f.u64(v + 24), 0);
    }
    assert_eq!(f.u64(values) as i64, -3);
    assert_eq!(f64::from_bits(f.u64(values + 32)), 0.5);

    // Updates go straight to the file.
    requests.set_val(MetricType::I64(42));
    latency.set_val(MetricType::F64(1.25));
    let f = file.read();
    assert_eq!(f.u64(values), 42);
    assert_eq!(f64::from_bits(f.u64(values + 32)), 1.25);
    assert_eq!(requests.val(), MetricType::I64(42));
}

#[test]
fn replaces_existing_file() {
    let file = TempMmv::new("replace");
    fs::write(&file.0, vec![0xff; 10_000]).unwrap();
    Mmv::new(&file.0, MmvFlags::empty(), 0)
        .map(&mut [&mut counter("a", 1)])
        .unwrap();
    assert_eq!(
        fs::metadata(&file.0).unwrap().len(),
        88 + 104 + 32 + 2 * 256
    );
}

#[test]
fn units_match_pmunits_bitfields() {
    assert_eq!(Units::NONE.bits(), 0);
    assert_eq!(Units::new(1, 0, 0, 2, 0, 0).bits(), 0x1002_0000); // MiB
    assert_eq!(Units::new(0, -1, 1, 0, 3, 0).bits(), 0x0f10_3000); // count/sec
    assert_eq!(Units::new(0, 0, 1, 0, 0, -2).bits(), 0x0010_0e00); // count x 10^-2
}

#[test]
fn rejects_invalid_files() {
    let file = TempMmv::new("invalid");
    let mmv = Mmv::new(&file.0, MmvFlags::empty(), 0);
    let err = |metrics: &mut [&mut Metric]| mmv.map(metrics).unwrap_err().kind();

    assert_eq!(err(&mut []), io::ErrorKind::InvalidInput);
    assert_eq!(
        err(&mut [&mut counter("a", 1), &mut counter("a", 2)]),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        err(&mut [&mut counter("a", 1), &mut counter("b", 1)]),
        io::ErrorKind::InvalidInput
    );
    let big_cluster = Mmv::new(&file.0, MmvFlags::empty(), 4096);
    assert_eq!(
        big_cluster
            .map(&mut [&mut counter("a", 1)])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(!file.0.exists());
}

#[test]
#[should_panic(expected = "not yet mapped")]
fn set_val_needs_a_mapping() {
    counter("a", 1).set_val(MetricType::I64(1));
}

#[test]
#[should_panic(expected = "wrong metric type")]
fn set_val_keeps_the_type() {
    let file = TempMmv::new("type");
    let mut metric = counter("a", 1);
    Mmv::new(&file.0, MmvFlags::empty(), 0)
        .map(&mut [&mut metric])
        .unwrap();
    metric.set_val(MetricType::F64(1.0));
}

#[test]
#[should_panic(expected = "longer than 63 bytes")]
fn rejects_long_names() {
    counter(&"x".repeat(64), 1);
}
