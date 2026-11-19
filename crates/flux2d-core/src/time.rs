/// A fixed-rate simulation clock.
///
/// Feed it the real time since the last frame with [`advance`](Self::advance); it returns how
/// many fixed steps to run. After the steps, [`alpha`](Self::alpha) says how far the clock is
/// between the last step and the next one (0..1), for interpolating what is drawn.
///
/// A slow frame never runs more than `max_steps` steps, so a long hitch (a breakpoint, a window
/// drag) can't snowball into a "spiral of death"; the leftover time is dropped instead.
#[derive(Debug, Clone)]
pub struct FixedTimestep {
    step: f64,
    accumulator: f64,
    max_steps: u32,
    total_steps: u64,
}

impl FixedTimestep {
    /// A clock that runs `hz` steps per second.
    pub fn new(hz: f64) -> Self {
        assert!(hz > 0.0, "step rate must be positive");
        Self { step: 1.0 / hz, accumulator: 0.0, max_steps: 8, total_steps: 0 }
    }

    /// The most steps one frame may run (default 8).
    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    /// Seconds per step.
    pub fn step_seconds(&self) -> f64 {
        self.step
    }

    /// Steps run since the clock started.
    pub fn total_steps(&self) -> u64 {
        self.total_steps
    }

    /// Adds `frame_seconds` of real time and returns the number of steps to run now.
    pub fn advance(&mut self, frame_seconds: f64) -> u32 {
        self.accumulator += frame_seconds.max(0.0);
        let mut steps = 0;
        while self.accumulator >= self.step && steps < self.max_steps {
            self.accumulator -= self.step;
            steps += 1;
        }
        if steps == self.max_steps && self.accumulator >= self.step {
            // Falling behind: drop the backlog rather than trying to catch up forever.
            self.accumulator %= self.step;
        }
        self.total_steps += u64::from(steps);
        steps
    }

    /// How far between the last step and the next one the clock is, 0..1.
    pub fn alpha(&self) -> f32 {
        ((self.accumulator / self.step) as f32).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_one_step_per_step_of_time() {
        let mut t = FixedTimestep::new(60.0);
        assert_eq!(t.advance(1.0 / 60.0 + 1e-9), 1);
        assert_eq!(t.advance(0.5 / 60.0), 0);
        assert!((t.alpha() - 0.5).abs() < 1e-3);
        assert_eq!(t.advance(0.5 / 60.0), 1);
        assert_eq!(t.total_steps(), 2);
    }

    #[test]
    fn slow_frame_is_capped() {
        let mut t = FixedTimestep::new(60.0).with_max_steps(4);
        assert_eq!(t.advance(1.0), 4);
        assert!(t.alpha() <= 1.0);
        // The backlog was dropped, so the next normal frame runs one step, not dozens.
        assert!(t.advance(1.0 / 60.0) <= 2);
    }

    #[test]
    fn many_small_frames_add_up() {
        let mut t = FixedTimestep::new(60.0);
        let steps: u32 = (0..240).map(|_| t.advance(1.0 / 240.0)).sum();
        assert!((59..=60).contains(&steps), "{steps}");
    }
}
