//! Flux2D core: the pieces of the game loop that don't touch the GPU.
//!
//! - [`FixedTimestep`]: a 60 Hz (or any rate) simulation clock with an interpolation factor
//!   for drawing between steps.
//! - [`Input`]: keyboard, mouse and gamepad state, with "pressed" edges that work both in fixed
//!   updates and in per-frame code. [`GamepadPoller`] feeds gamepads in through gilrs
//!   (`gamepad` feature, on by default).
//! - [`FrameStats`]: frame time history with average, p95, p99 and max, the same fields the
//!   Cosmic Commando `PERF SUMMARY` prints, so the two can be compared.
//! - [`Rng`]: a small, fast, seedable random number generator.
//! - [`profile_scope!`] and [`profile_frame!`]: Tracy profiler zones (`profile` feature; they
//!   compile to nothing without it).

mod input;
mod rng;
mod stats;
mod time;

#[cfg(feature = "gamepad")]
pub use input::GamepadPoller;
pub use input::{
    Gamepad, GamepadAxis, GamepadButton, Input, KeyCode, MAX_GAMEPADS, MouseButton, STICK_DEADZONE,
};
pub use rng::Rng;
pub use stats::{FrameStats, Summary};
pub use time::FixedTimestep;

#[cfg(feature = "profile")]
#[doc(hidden)]
pub use tracy_client as __tracy;

/// Starts the Tracy client (with the `profile` feature). The app loop calls this; call it
/// yourself only when profiling code outside [`flux2d::run`].
pub fn start_profiler() {
    #[cfg(feature = "profile")]
    {
        let _ = tracy_client::Client::start();
    }
}

/// Times the rest of the enclosing block as a Tracy zone named `$name` (a string literal).
///
/// Needs the `profile` feature (`cargo run --release --features profile`) and a running
/// [Tracy](https://github.com/wolfpld/tracy) profiler to connect; otherwise it is nothing.
#[cfg(feature = "profile")]
#[macro_export]
macro_rules! profile_scope {
    ($name:literal) => {
        let _flux2d_profile_span =
            $crate::__tracy::Client::running().map(|c| c.span($crate::__tracy::span_location!($name), 0));
    };
}

#[cfg(not(feature = "profile"))]
#[macro_export]
macro_rules! profile_scope {
    ($name:literal) => {};
}

/// Marks the end of a frame for Tracy (the app loop does this once per displayed frame).
#[cfg(feature = "profile")]
#[macro_export]
macro_rules! profile_frame {
    () => {
        if let Some(c) = $crate::__tracy::Client::running() {
            c.frame_mark();
        }
    };
}

#[cfg(not(feature = "profile"))]
#[macro_export]
macro_rules! profile_frame {
    () => {};
}
