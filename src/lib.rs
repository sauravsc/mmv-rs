//! A proof-of-concept, pure Rust writer for [Performance Co-Pilot] (PCP)
//! Memory Mapped Value (MMV) version 1 files.
//!
//! **This crate is a 2017 proof of concept.** For production use, see
//! [hornet](https://github.com/performancecopilot/hornet), which grew out of
//! it.
//!
//! ```no_run
//! use mmv::{Metric, MetricSem, MetricType, Mmv, MmvFlags, Units};
//!
//! let mut requests = Metric::new(
//!     "requests", 1, MetricSem::Counter, 0, Units::new(0, 0, 1, 0, 0, 0),
//!     MetricType::I64(0), "Requests", "Number of requests served",
//! );
//! // PCP's MMV agent reads files in $PCP_TMP_DIR/mmv.
//! Mmv::new("/var/lib/pcp/tmp/mmv/myapp", MmvFlags::empty(), 1).map(&mut [&mut requests])?;
//! requests.set_val(MetricType::I64(1));
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! [Performance Co-Pilot]: https://pcp.io

#![warn(missing_docs)]

use memmap2::MmapRaw;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const HDR_LEN: usize = 40;
const TOC_BLOCK_LEN: usize = 16;
const METRIC_BLOCK_LEN: usize = 104;
const VALUE_BLOCK_LEN: usize = 32;
const STRING_BLOCK_LEN: usize = 256;
const METRIC_NAME_MAX_LEN: usize = 64;

const N_TOC_BLOCKS: usize = 3;
const TOC_METRICS: u32 = 3;
const TOC_VALUES: u32 = 4;
const TOC_STRINGS: u32 = 5;

const MMV_TYPE_I64: u32 = 2;
const MMV_TYPE_DOUBLE: u32 = 5;

/// Offset of the second generation number in the header.
const GEN2_OFFSET: usize = 16;

bitflags::bitflags! {
    /// Flags for an MMV file.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct MmvFlags: u32 {
        /// Don't prefix metric names with the MMV file name.
        const NOPREFIX = 1;
        /// Stop reporting metrics once the writing process has exited.
        const PROCESS = 2;
        /// Treat sentinel values as "no value available".
        const SENTINEL = 4;
    }
}

/// The semantics of a metric's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetricSem {
    /// A monotonically increasing counter.
    Counter = 1,
    /// An instantaneous value that can go up or down.
    Instant = 3,
    /// A value that changes rarely, if ever.
    Discrete = 4,
}

/// The type and value of a metric.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricType {
    /// A 64-bit signed integer.
    I64(i64),
    /// A 64-bit floating point number.
    F64(f64),
}

impl MetricType {
    fn code(self) -> u32 {
        match self {
            MetricType::I64(_) => MMV_TYPE_I64,
            MetricType::F64(_) => MMV_TYPE_DOUBLE,
        }
    }

    fn bits(self) -> u64 {
        match self {
            MetricType::I64(x) => x as u64,
            MetricType::F64(x) => x.to_bits(),
        }
    }
}

/// The dimension and scale of a metric, PCP's `pmUnits`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Units(u32);

impl Units {
    /// No dimension.
    pub const NONE: Units = Units(0);

    /// Creates units from dimensions and scales, like PCP's `MMV_UNITS`
    /// macro.
    ///
    /// Dimensions range from -8 to 7. Scales are the values of PCP's
    /// `PM_SPACE_*`, `PM_TIME_*` and `PM_COUNT_*` constants: for example,
    /// microseconds are `Units::new(0, 1, 0, 0, 1, 0)` (`PM_TIME_USEC` is 1)
    /// and a plain count is `Units::new(0, 0, 1, 0, 0, 0)`.
    pub const fn new(
        dim_space: i8,
        dim_time: i8,
        dim_count: i8,
        scale_space: u8,
        scale_time: u8,
        scale_count: i8,
    ) -> Units {
        const fn nibble(x: i8) -> u32 {
            (x as u32) & 0xf
        }
        Units(
            nibble(dim_space) << 28
                | nibble(dim_time) << 24
                | nibble(dim_count) << 20
                | nibble(scale_space as i8) << 16
                | nibble(scale_time as i8) << 12
                | nibble(scale_count) << 8,
        )
    }

    /// The units as they are stored in an MMV file.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Where a mapped metric's value lives.
#[derive(Debug)]
struct Slot {
    map: Arc<MmapRaw>,
    offset: usize,
}

/// A metric, which can be written to an MMV file with [`Mmv::map`].
#[derive(Debug)]
pub struct Metric {
    name: String,
    item: u32,
    sem: MetricSem,
    indom: u32,
    units: Units,
    shorttext: String,
    longtext: String,
    val: MetricType,
    slot: Option<Slot>,
}

impl Metric {
    /// Creates a metric with an initial value.
    ///
    /// `item` must be unique within an MMV file. MMV v1 files written by this
    /// crate have no instance domains, so `indom` should be 0.
    ///
    /// # Panics
    ///
    /// Panics if `name` is empty, longer than 63 bytes or contains a NUL
    /// byte, or if a help text is longer than 255 bytes or contains a NUL
    /// byte.
    #[allow(clippy::too_many_arguments)] // mirrors the fields of an MMV metric
    pub fn new(
        name: &str,
        item: u32,
        sem: MetricSem,
        indom: u32,
        units: Units,
        init_val: MetricType,
        shorthelp: &str,
        longhelp: &str,
    ) -> Self {
        assert!(!name.is_empty(), "metric name is empty");
        assert!(
            name.len() < METRIC_NAME_MAX_LEN,
            "metric name {name:?} is longer than {} bytes",
            METRIC_NAME_MAX_LEN - 1
        );
        for text in [shorthelp, longhelp] {
            assert!(
                text.len() < STRING_BLOCK_LEN,
                "help text {text:?} is longer than {} bytes",
                STRING_BLOCK_LEN - 1
            );
        }
        for text in [name, shorthelp, longhelp] {
            assert!(!text.contains('\0'), "{text:?} contains a NUL byte");
        }

        Metric {
            name: name.to_owned(),
            item,
            sem,
            indom,
            units,
            shorttext: shorthelp.to_owned(),
            longtext: longhelp.to_owned(),
            val: init_val,
            slot: None,
        }
    }

    /// The metric's current value.
    pub fn val(&self) -> MetricType {
        self.val
    }

    /// Sets the metric's value in the MMV file.
    ///
    /// # Panics
    ///
    /// Panics if the metric has not been mapped with [`Mmv::map`], or if
    /// `new_val` has a different type from the metric's initial value.
    pub fn set_val(&mut self, new_val: MetricType) {
        assert_eq!(
            self.val.code(),
            new_val.code(),
            "wrong metric type for {}",
            self.name
        );
        let slot = self.slot.as_ref().expect("metric not yet mapped");
        // SAFETY: the slot is an 8-byte aligned value inside the mapping,
        // which `slot.map` keeps alive. Only this metric writes to it, and
        // `&mut self` rules out concurrent writes. Volatile, because other
        // processes read the mapping.
        unsafe {
            let ptr = slot.map.as_mut_ptr().add(slot.offset).cast::<u64>();
            ptr.write_volatile(new_val.bits());
        }
        self.val = new_val;
    }
}

/// An MMV file to write metrics to.
#[derive(Clone, Debug)]
pub struct Mmv {
    path: PathBuf,
    flags: MmvFlags,
    cluster_id: u32,
}

/// Writes `value` in native byte order at `offset`.
fn put(buf: &mut [u8], offset: usize, value: &[u8]) {
    buf[offset..offset + value.len()].copy_from_slice(value);
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl Mmv {
    /// Describes an MMV file at `path`.
    ///
    /// PCP's MMV agent reads files in `$PCP_TMP_DIR/mmv`, which is usually
    /// `/var/lib/pcp/tmp/mmv`. `cluster_id` must be less than 4096.
    pub fn new(path: impl AsRef<Path>, flags: MmvFlags, cluster_id: u32) -> Mmv {
        Mmv {
            path: path.as_ref().to_owned(),
            flags,
            cluster_id,
        }
    }

    /// Writes `metrics` to a new MMV file, replacing any existing file, and
    /// maps it so that [`Metric::set_val`] updates the file.
    ///
    /// # Errors
    ///
    /// Fails if there are no metrics, if two metrics share a name or item,
    /// if the cluster ID is 4096 or more, or if the file cannot be created
    /// or mapped.
    pub fn map(&self, metrics: &mut [&mut Metric]) -> io::Result<()> {
        if metrics.is_empty() {
            return Err(invalid("an MMV file needs at least one metric".into()));
        }
        if self.cluster_id >= 1 << 12 {
            return Err(invalid(format!(
                "cluster ID {} is not less than 4096",
                self.cluster_id
            )));
        }
        let mut names = HashSet::new();
        let mut items = HashSet::new();
        for m in metrics.iter() {
            if !names.insert(&m.name) {
                return Err(invalid(format!("duplicate metric name {:?}", m.name)));
            }
            if !items.insert(m.item) {
                return Err(invalid(format!("duplicate metric item {}", m.item)));
            }
        }

        let contents = self.contents(metrics);
        let file = self.create(&contents)?;
        let map = Arc::new(MmapRaw::map_raw(&file)?);

        // Mark the file as complete by making the generation numbers match.
        let gen2 = &contents[8..16];
        // SAFETY: the mapping is at least as long as the header.
        unsafe {
            std::ptr::copy_nonoverlapping(
                gen2.as_ptr(),
                map.as_mut_ptr().add(GEN2_OFFSET),
                gen2.len(),
            );
        }

        let values = value_section_offset(metrics.len());
        for (i, m) in metrics.iter_mut().enumerate() {
            m.slot = Some(Slot {
                map: Arc::clone(&map),
                offset: values + i * VALUE_BLOCK_LEN,
            });
        }
        Ok(())
    }

    /// Creates the file with `contents`, the way `libpcp_mmv` does: removing
    /// any old file first makes PCP's MMV agent reload it.
    fn create(&self, contents: &[u8]) -> io::Result<File> {
        if let Err(err) = fs::remove_file(&self.path) {
            if err.kind() != io::ErrorKind::NotFound {
                return Err(err);
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o644);
        let mut file = options.open(&self.path)?;
        file.write_all(contents)?;
        Ok(file)
    }

    /// The MMV v1 file for `metrics`, with the second generation number
    /// still zero.
    fn contents(&self, metrics: &[&mut Metric]) -> Vec<u8> {
        let n = metrics.len();
        let metric_section = HDR_LEN + N_TOC_BLOCKS * TOC_BLOCK_LEN;
        let value_section = value_section_offset(n);
        let string_section = value_section + n * VALUE_BLOCK_LEN;
        let mut buf = vec![0; string_section + 2 * n * STRING_BLOCK_LEN];

        // Header.
        let generation = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        put(&mut buf, 0, b"MMV\0");
        put(&mut buf, 4, &1u32.to_ne_bytes()); // version
        put(&mut buf, 8, &generation.to_ne_bytes());
        // The second generation number is written once the file is mapped.
        put(&mut buf, 24, &(N_TOC_BLOCKS as i32).to_ne_bytes());
        put(&mut buf, 28, &self.flags.bits().to_ne_bytes());
        put(&mut buf, 32, &(std::process::id() as i32).to_ne_bytes());
        put(&mut buf, 36, &self.cluster_id.to_ne_bytes());

        // Table of contents: type, number of entries, section offset.
        let tocs = [
            (TOC_METRICS, n, metric_section),
            (TOC_VALUES, n, value_section),
            (TOC_STRINGS, 2 * n, string_section),
        ];
        for (i, (toc_type, count, offset)) in tocs.into_iter().enumerate() {
            let toc = HDR_LEN + i * TOC_BLOCK_LEN;
            put(&mut buf, toc, &toc_type.to_ne_bytes());
            put(&mut buf, toc + 4, &(count as u32).to_ne_bytes());
            put(&mut buf, toc + 8, &(offset as u64).to_ne_bytes());
        }

        for (i, m) in metrics.iter().enumerate() {
            let metric = metric_section + i * METRIC_BLOCK_LEN;
            let value = value_section + i * VALUE_BLOCK_LEN;
            let shorttext = string_section + 2 * i * STRING_BLOCK_LEN;
            let longtext = shorttext + STRING_BLOCK_LEN;

            // Metric block. The name is NUL-padded to 64 bytes.
            put(&mut buf, metric, m.name.as_bytes());
            let fields = [
                m.item,
                m.val.code(),
                m.sem as u32,
                m.units.bits(),
                m.indom,
                0, // padding
            ];
            for (j, field) in fields.into_iter().enumerate() {
                put(
                    &mut buf,
                    metric + METRIC_NAME_MAX_LEN + 4 * j,
                    &field.to_ne_bytes(),
                );
            }
            put(&mut buf, metric + 88, &(shorttext as u64).to_ne_bytes());
            put(&mut buf, metric + 96, &(longtext as u64).to_ne_bytes());

            // Value block: value, extra, metric offset, instance offset.
            put(&mut buf, value, &m.val.bits().to_ne_bytes());
            put(&mut buf, value + 16, &(metric as u64).to_ne_bytes());

            // String blocks, NUL-padded.
            put(&mut buf, shorttext, m.shorttext.as_bytes());
            put(&mut buf, longtext, m.longtext.as_bytes());
        }
        buf
    }
}

fn value_section_offset(n_metrics: usize) -> usize {
    HDR_LEN + N_TOC_BLOCKS * TOC_BLOCK_LEN + n_metrics * METRIC_BLOCK_LEN
}
