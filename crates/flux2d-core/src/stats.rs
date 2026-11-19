/// Frame time history for benchmarks and the window title readout.
///
/// Times are pushed in milliseconds. [`summary`](Self::summary) gives the same numbers the Cosmic
/// Commando `PERF SUMMARY` line prints (average, p95, p99, max and the share of frames slower
/// than 16.7 ms), so a Flux2D run and a Godot run can be compared field by field.
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    samples: Vec<f32>,
}

/// A summary of a run of frame times, all in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub avg_ms: f32,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub max_ms: f32,
    /// Share of frames over 16.7 ms (a missed 60 Hz frame), 0..1.
    pub over_60hz: f32,
}

impl Summary {
    /// Average frames per second.
    pub fn fps(&self) -> f32 {
        if self.avg_ms > 0.0 { 1000.0 / self.avg_ms } else { 0.0 }
    }
}

impl FrameStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ms: f32) {
        self.samples.push(ms);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Summary of every sample pushed so far; `None` when empty.
    pub fn summary(&self) -> Option<Summary> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len();
        let pct = |p: f32| sorted[(((n - 1) as f32) * p).round() as usize];
        let sum: f64 = sorted.iter().map(|&v| f64::from(v)).sum();
        Some(Summary {
            frames: n,
            avg_ms: (sum / n as f64) as f32,
            p50_ms: pct(0.50),
            p95_ms: pct(0.95),
            p99_ms: pct(0.99),
            max_ms: sorted[n - 1],
            over_60hz: sorted.iter().filter(|&&v| v > 1000.0 / 60.0).count() as f32 / n as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_of_known_values() {
        let mut s = FrameStats::new();
        for i in 1..=100 {
            s.push(i as f32);
        }
        let sum = s.summary().unwrap();
        assert_eq!(sum.frames, 100);
        assert!((sum.avg_ms - 50.5).abs() < 1e-4);
        assert_eq!(sum.max_ms, 100.0);
        assert!((sum.p95_ms - 95.0).abs() <= 1.0);
        assert!((sum.p99_ms - 99.0).abs() <= 1.0);
        assert!((sum.over_60hz - 0.84).abs() < 1e-4);
    }

    #[test]
    fn empty_has_no_summary() {
        assert!(FrameStats::new().summary().is_none());
    }
}
