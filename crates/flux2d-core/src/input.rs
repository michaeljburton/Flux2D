use glam::Vec2;
use std::collections::HashSet;
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

/// A gamepad button, named by position (Xbox layout: South = A, East = B, West = X, North = Y).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GamepadButton {
    South,
    East,
    North,
    West,
    LeftBumper,
    RightBumper,
    /// Also readable as an axis ([`GamepadAxis::LeftTrigger`]); "down" past half way.
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Guide,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

/// A gamepad axis. Sticks are -1..1 with **y down** (pushing up is negative, like screen and
/// world coordinates); triggers are 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

/// Gamepads tracked at once.
pub const MAX_GAMEPADS: usize = 4;
/// Stick movement ignored round the centre (drift), as a share of full travel.
pub const STICK_DEADZONE: f32 = 0.18;

/// One gamepad's state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gamepad {
    pub connected: bool,
    down: u32,
    axes: [f32; 6],
}

impl Gamepad {
    pub fn button_down(&self, b: GamepadButton) -> bool {
        self.down & bit(b) != 0
    }

    /// Raw axis value, no deadzone.
    pub fn axis(&self, a: GamepadAxis) -> f32 {
        self.axes[a as usize]
    }

    /// Left stick with a radial deadzone, rescaled so it still reaches full length; y down.
    pub fn left_stick(&self) -> Vec2 {
        deadzone(Vec2::new(self.axis(GamepadAxis::LeftX), self.axis(GamepadAxis::LeftY)))
    }

    pub fn right_stick(&self) -> Vec2 {
        deadzone(Vec2::new(self.axis(GamepadAxis::RightX), self.axis(GamepadAxis::RightY)))
    }
}

fn bit(b: GamepadButton) -> u32 {
    1 << b as u32
}

fn deadzone(v: Vec2) -> Vec2 {
    let len = v.length();
    if len <= STICK_DEADZONE {
        Vec2::ZERO
    } else {
        v / len * ((len - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).min(1.0)
    }
}

/// Things that changed since an edge set was last cleared.
#[derive(Debug, Default)]
struct Edges {
    pressed: HashSet<KeyCode>,
    released: HashSet<KeyCode>,
    mouse_pressed: HashSet<MouseButton>,
    mouse_released: HashSet<MouseButton>,
    pad_pressed: [u32; MAX_GAMEPADS],
    pad_released: [u32; MAX_GAMEPADS],
}

impl Edges {
    fn clear(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.mouse_pressed.clear();
        self.mouse_released.clear();
        self.pad_pressed = [0; MAX_GAMEPADS];
        self.pad_released = [0; MAX_GAMEPADS];
    }
}

/// Keyboard, mouse and gamepad state.
///
/// "Down" means held. "Pressed" and "released" are edges: true once per change. They are
/// tracked twice, so they work wherever they are read:
///
/// - In `Game::update` (a fixed step), an edge is seen by the **first step after it
///   happened**, even if the frame it happened in ran no step at all (a 240 Hz display) or
///   several.
/// - In `Game::draw` and elsewhere, an edge lasts **one displayed frame**.
///
/// The app loop calls [`begin_step`](Self::begin_step) / [`end_step`](Self::end_step) round
/// each update and [`end_frame`](Self::end_frame) after each frame.
#[derive(Debug, Default)]
pub struct Input {
    down: HashSet<KeyCode>,
    mouse_down: HashSet<MouseButton>,
    frame: Edges,
    step: Edges,
    in_step: bool,
    mouse_pos: Vec2,
    mouse_delta: Vec2,
    wheel: f32,
    pads: [Gamepad; MAX_GAMEPADS],
}

impl Input {
    #[inline]
    fn edges(&self) -> &Edges {
        if self.in_step { &self.step } else { &self.frame }
    }

    pub fn key_down(&self, key: KeyCode) -> bool {
        self.down.contains(&key)
    }
    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.edges().pressed.contains(&key)
    }
    pub fn key_released(&self, key: KeyCode) -> bool {
        self.edges().released.contains(&key)
    }
    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_down.contains(&button)
    }
    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.edges().mouse_pressed.contains(&button)
    }
    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.edges().mouse_released.contains(&button)
    }
    /// Pointer position in window pixels (top-left origin).
    pub fn mouse_position(&self) -> Vec2 {
        self.mouse_pos
    }
    /// Pointer movement this frame, in window pixels.
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }
    /// Wheel movement this frame in "lines" (positive = away from the user).
    pub fn wheel(&self) -> f32 {
        self.wheel
    }

    /// Gamepad `index` (0..[`MAX_GAMEPADS`]), connected or not.
    pub fn gamepad(&self, index: usize) -> &Gamepad {
        &self.pads[index]
    }

    /// The first connected gamepad, if any.
    pub fn first_gamepad(&self) -> Option<&Gamepad> {
        self.pads.iter().find(|p| p.connected)
    }

    /// Whether `button` is held on any gamepad.
    pub fn pad_down(&self, button: GamepadButton) -> bool {
        self.pads.iter().any(|p| p.button_down(button))
    }

    /// Whether `button` was pressed on any gamepad (an edge, like [`key_pressed`](Self::key_pressed)).
    pub fn pad_pressed(&self, button: GamepadButton) -> bool {
        self.edges().pad_pressed.iter().any(|&b| b & bit(button) != 0)
    }

    pub fn pad_released(&self, button: GamepadButton) -> bool {
        self.edges().pad_released.iter().any(|&b| b & bit(button) != 0)
    }

    /// The first connected gamepad's left stick (deadzoned, y down), or zero.
    pub fn left_stick(&self) -> Vec2 {
        self.first_gamepad().map_or(Vec2::ZERO, Gamepad::left_stick)
    }

    /// The first connected gamepad's right stick (deadzoned, y down), or zero.
    pub fn right_stick(&self) -> Vec2 {
        self.first_gamepad().map_or(Vec2::ZERO, Gamepad::right_stick)
    }

    /// An axis of the first connected gamepad, or zero.
    pub fn pad_axis(&self, axis: GamepadAxis) -> f32 {
        self.first_gamepad().map_or(0.0, |p| p.axis(axis))
    }

    pub fn set_key(&mut self, key: KeyCode, is_down: bool) {
        if is_down {
            if self.down.insert(key) {
                self.frame.pressed.insert(key);
                self.step.pressed.insert(key);
            }
        } else if self.down.remove(&key) {
            self.frame.released.insert(key);
            self.step.released.insert(key);
        }
    }

    pub fn set_mouse_button(&mut self, button: MouseButton, is_down: bool) {
        if is_down {
            if self.mouse_down.insert(button) {
                self.frame.mouse_pressed.insert(button);
                self.step.mouse_pressed.insert(button);
            }
        } else if self.mouse_down.remove(&button) {
            self.frame.mouse_released.insert(button);
            self.step.mouse_released.insert(button);
        }
    }

    pub fn set_mouse_position(&mut self, pos: Vec2) {
        self.mouse_delta += pos - self.mouse_pos;
        self.mouse_pos = pos;
    }

    pub fn add_wheel(&mut self, lines: f32) {
        self.wheel += lines;
    }

    pub fn set_gamepad_connected(&mut self, pad: usize, connected: bool) {
        if pad >= MAX_GAMEPADS {
            return;
        }
        if !connected {
            // Release whatever was held so nothing sticks down.
            let held = self.pads[pad].down;
            self.frame.pad_released[pad] |= held;
            self.step.pad_released[pad] |= held;
            self.pads[pad] = Gamepad::default();
        }
        self.pads[pad].connected = connected;
    }

    pub fn set_gamepad_button(&mut self, pad: usize, button: GamepadButton, is_down: bool) {
        let Some(p) = self.pads.get_mut(pad) else { return };
        let b = bit(button);
        let was = p.down & b != 0;
        if is_down && !was {
            p.down |= b;
            self.frame.pad_pressed[pad] |= b;
            self.step.pad_pressed[pad] |= b;
        } else if !is_down && was {
            p.down &= !b;
            self.frame.pad_released[pad] |= b;
            self.step.pad_released[pad] |= b;
        }
    }

    /// Sets an axis. Sticks take y down (flip a y-up source first); triggers 0..1 also set
    /// their button past half way.
    pub fn set_gamepad_axis(&mut self, pad: usize, axis: GamepadAxis, value: f32) {
        let Some(p) = self.pads.get_mut(pad) else { return };
        p.axes[axis as usize] = value;
        match axis {
            GamepadAxis::LeftTrigger => self.set_gamepad_button(pad, GamepadButton::LeftTrigger, value > 0.5),
            GamepadAxis::RightTrigger => {
                self.set_gamepad_button(pad, GamepadButton::RightTrigger, value > 0.5)
            }
            _ => {}
        }
    }

    /// Releases everything held, e.g. when the window loses focus.
    pub fn release_all(&mut self) {
        let keys: Vec<_> = self.down.drain().collect();
        for k in keys {
            self.frame.released.insert(k);
            self.step.released.insert(k);
        }
        let buttons: Vec<_> = self.mouse_down.drain().collect();
        for b in buttons {
            self.frame.mouse_released.insert(b);
            self.step.mouse_released.insert(b);
        }
    }

    /// Called by the app loop before each fixed update: edges now read as "since the last step".
    pub fn begin_step(&mut self) {
        self.in_step = true;
    }

    /// Called by the app loop after each fixed update: clears the step edges.
    pub fn end_step(&mut self) {
        self.step.clear();
        self.in_step = false;
    }

    /// Clears the per-frame edges and deltas. Called by the app loop after each frame.
    pub fn end_frame(&mut self) {
        self.frame.clear();
        self.mouse_delta = Vec2::ZERO;
        self.wheel = 0.0;
    }
}

/// Reads gamepads through [gilrs](https://docs.rs/gilrs) into an [`Input`].
#[cfg(feature = "gamepad")]
pub struct GamepadPoller {
    gilrs: Option<gilrs::Gilrs>,
    /// gilrs gamepad id for each of our slots.
    slots: [Option<gilrs::GamepadId>; MAX_GAMEPADS],
}

#[cfg(feature = "gamepad")]
impl GamepadPoller {
    /// Starts listening for gamepads. Without gamepad support on this system, polling does
    /// nothing.
    pub fn new(input: &mut Input) -> Self {
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                log::warn!("gamepads unavailable: {e}");
                None
            }
        };
        let mut poller = Self { gilrs, slots: [None; MAX_GAMEPADS] };
        let ids: Vec<_> = poller.gilrs.iter().flat_map(|g| g.gamepads().map(|(id, _)| id)).collect();
        for id in ids {
            poller.connect(input, id);
        }
        poller
    }

    fn connect(&mut self, input: &mut Input, id: gilrs::GamepadId) -> Option<usize> {
        if let Some(slot) = self.slots.iter().position(|s| *s == Some(id)) {
            return Some(slot);
        }
        let slot = self.slots.iter().position(Option::is_none)?;
        self.slots[slot] = Some(id);
        input.set_gamepad_connected(slot, true);
        if let Some(g) = &self.gilrs {
            log::info!("gamepad {slot}: {}", g.gamepad(id).name());
        }
        Some(slot)
    }

    /// Applies every gamepad event since the last poll. The app loop calls this each frame.
    pub fn poll(&mut self, input: &mut Input) {
        use gilrs::{Axis, Button, EventType};
        while let Some(event) = self.gilrs.as_mut().and_then(gilrs::Gilrs::next_event) {
            let slot = match event.event {
                EventType::Disconnected => {
                    if let Some(slot) = self.slots.iter().position(|s| *s == Some(event.id)) {
                        self.slots[slot] = None;
                        input.set_gamepad_connected(slot, false);
                    }
                    continue;
                }
                _ => match self.connect(input, event.id) {
                    Some(s) => s,
                    None => continue, // more pads than slots
                },
            };
            let button = |b: Button| {
                Some(match b {
                    Button::South => GamepadButton::South,
                    Button::East => GamepadButton::East,
                    Button::North => GamepadButton::North,
                    Button::West => GamepadButton::West,
                    Button::LeftTrigger => GamepadButton::LeftBumper,
                    Button::RightTrigger => GamepadButton::RightBumper,
                    Button::LeftTrigger2 => GamepadButton::LeftTrigger,
                    Button::RightTrigger2 => GamepadButton::RightTrigger,
                    Button::Select => GamepadButton::Select,
                    Button::Start => GamepadButton::Start,
                    Button::Mode => GamepadButton::Guide,
                    Button::LeftThumb => GamepadButton::LeftStick,
                    Button::RightThumb => GamepadButton::RightStick,
                    Button::DPadUp => GamepadButton::DPadUp,
                    Button::DPadDown => GamepadButton::DPadDown,
                    Button::DPadLeft => GamepadButton::DPadLeft,
                    Button::DPadRight => GamepadButton::DPadRight,
                    _ => return None,
                })
            };
            match event.event {
                EventType::ButtonPressed(b, _) => {
                    if let Some(b) = button(b) {
                        input.set_gamepad_button(slot, b, true);
                    }
                }
                EventType::ButtonReleased(b, _) => {
                    if let Some(b) = button(b) {
                        input.set_gamepad_button(slot, b, false);
                    }
                }
                EventType::ButtonChanged(Button::LeftTrigger2, v, _) => {
                    input.set_gamepad_axis(slot, GamepadAxis::LeftTrigger, v);
                }
                EventType::ButtonChanged(Button::RightTrigger2, v, _) => {
                    input.set_gamepad_axis(slot, GamepadAxis::RightTrigger, v);
                }
                EventType::AxisChanged(axis, v, _) => {
                    let (axis, v) = match axis {
                        Axis::LeftStickX => (GamepadAxis::LeftX, v),
                        Axis::LeftStickY => (GamepadAxis::LeftY, -v), // gilrs is y up
                        Axis::RightStickX => (GamepadAxis::RightX, v),
                        Axis::RightStickY => (GamepadAxis::RightY, -v),
                        Axis::LeftZ => (GamepadAxis::LeftTrigger, v.max(0.0)),
                        Axis::RightZ => (GamepadAxis::RightTrigger, v.max(0.0)),
                        _ => continue,
                    };
                    input.set_gamepad_axis(slot, axis, v);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_is_one_frame_only() {
        let mut i = Input::default();
        i.set_key(KeyCode::Space, true);
        assert!(i.key_pressed(KeyCode::Space) && i.key_down(KeyCode::Space));
        i.end_frame();
        i.set_key(KeyCode::Space, true); // key repeat
        assert!(!i.key_pressed(KeyCode::Space) && i.key_down(KeyCode::Space));
        i.set_key(KeyCode::Space, false);
        assert!(i.key_released(KeyCode::Space) && !i.key_down(KeyCode::Space));
    }

    #[test]
    fn step_edges_survive_frames_without_steps() {
        let mut i = Input::default();
        i.set_key(KeyCode::KeyF, true);
        // A fast display: this frame ran no fixed step.
        i.end_frame();
        assert!(!i.key_pressed(KeyCode::KeyF), "frame edge is gone");
        // The next frame runs two steps: only the first sees the press.
        i.begin_step();
        assert!(i.key_pressed(KeyCode::KeyF));
        i.end_step();
        i.begin_step();
        assert!(!i.key_pressed(KeyCode::KeyF));
        i.end_step();
        assert!(i.key_down(KeyCode::KeyF));
    }

    #[test]
    fn mouse_delta_accumulates() {
        let mut i = Input::default();
        i.set_mouse_position(Vec2::new(10.0, 10.0));
        i.end_frame();
        i.set_mouse_position(Vec2::new(15.0, 12.0));
        i.set_mouse_position(Vec2::new(20.0, 10.0));
        assert_eq!(i.mouse_delta(), Vec2::new(10.0, 0.0));
    }

    #[test]
    fn gamepad_buttons_sticks_and_disconnect() {
        let mut i = Input::default();
        assert!(i.first_gamepad().is_none());
        i.set_gamepad_connected(1, true);
        i.set_gamepad_button(1, GamepadButton::South, true);
        assert!(i.pad_pressed(GamepadButton::South) && i.pad_down(GamepadButton::South));
        i.end_frame();
        assert!(!i.pad_pressed(GamepadButton::South) && i.pad_down(GamepadButton::South));

        i.set_gamepad_axis(1, GamepadAxis::LeftX, 0.1);
        assert_eq!(i.left_stick(), Vec2::ZERO, "inside the deadzone");
        i.set_gamepad_axis(1, GamepadAxis::LeftX, 1.0);
        assert!((i.left_stick() - Vec2::X).length() < 1e-5);

        i.set_gamepad_axis(1, GamepadAxis::RightTrigger, 0.8);
        assert!(i.pad_down(GamepadButton::RightTrigger));
        assert_eq!(i.pad_axis(GamepadAxis::RightTrigger), 0.8);

        i.set_gamepad_connected(1, false);
        assert!(i.pad_released(GamepadButton::South) && !i.pad_down(GamepadButton::South));
        assert!(i.first_gamepad().is_none());
    }
}
