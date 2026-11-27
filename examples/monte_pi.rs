//! Estimates Pi with Monte Carlo trials, publishing the number of trials and
//! the current estimate as MMV metrics.
//!
//! Usage: `cargo run --example monte_pi PATH [TRIALS]`
//!
//! Writes the MMV file at `PATH` (for PCP to read it, use a file in
//! `$PCP_TMP_DIR/mmv`, usually `/var/lib/pcp/tmp/mmv`), then runs `TRIALS`
//! trials (default 1,000,000), one every 100ms.

use mmv::{Metric, MetricSem, MetricType, Mmv, MmvFlags, Units};
use std::env;
use std::thread;
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: monte_pi PATH [TRIALS]");
    let n_trials: i64 = args
        .next()
        .map_or(1_000_000, |n| n.parse().expect("TRIALS must be a number"));

    let mut trials = Metric::new(
        "trials",
        1,
        MetricSem::Counter,
        0,
        Units::new(0, 0, 1, 0, 0, 0),
        MetricType::I64(0),
        "Trials",
        "Number of Monte Carlo trials",
    );
    let mut pi = Metric::new(
        "pi",
        2,
        MetricSem::Instant,
        0,
        Units::NONE,
        MetricType::F64(0.0),
        "Estimated Pi",
        "Estimated value of Pi through Monte Carlo trials",
    );

    let mmv = Mmv::new(&path, MmvFlags::empty(), 0);
    mmv.map(&mut [&mut trials, &mut pi])?;

    let mut in_circle = 0;
    for i in 1..=n_trials {
        trials.set_val(MetricType::I64(i));

        let x: f64 = rand::random_range(-1.0..1.0);
        let y: f64 = rand::random_range(-1.0..1.0);
        if x * x + y * y <= 1.0 {
            in_circle += 1;
        }
        pi.set_val(MetricType::F64(in_circle as f64 / i as f64 * 4.0));

        if i < n_trials {
            thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}
