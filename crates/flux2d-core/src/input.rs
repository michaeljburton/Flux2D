use glam::Vec2;
use std::collections::HashSet;
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

/// Keyboard and mouse state for the current frame.
///
/// "Down" means held; "pressed" and "released" are true only on the frame the change happened.
/// The app loop calls [`end_frame`](Self::end_frame) after each frame to clear the edges.
#[derive(Debug, Default)]
pub struct Input {
    down: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    released: HashSet<KeyCode>,
    mouse_down: HashSet<MouseButton>,
    mouse_pressed: HashSet<MouseButton>,
    mouse_released: HashSet<MouseButton>,
    mouse_pos: Vec2,
    mouse_delta: Vec2,
    wheel: f32,
}

impl Input {
    pub fn key_down(&self, key: KeyCode) -> bool {
        self.down.contains(&key)
    }
    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }
    pub fn key_released(&self, key: KeyCode) -> bool {
        self.released.contains(&key)
    }
    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_down.contains(&button)
    }
    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.mouse_pressed.contains(&button)
    }
    pub fn mouse_released(&self, button: MouseButton) -> bool {
        self.mouse_released.contains(&button)
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

    pub fn set_key(&mut self, key: KeyCode, is_down: bool) {
        if is_down {
            if self.down.insert(key) {
                self.pressed.insert(key);
            }
        } else if self.down.remove(&key) {
            self.released.insert(key);
        }
    }

    pub fn set_mouse_button(&mut self, button: MouseButton, is_down: bool) {
        if is_down {
            if self.mouse_down.insert(button) {
                self.mouse_pressed.insert(button);
            }
        } else if self.mouse_down.remove(&button) {
            self.mouse_released.insert(button);
        }
    }

    pub fn set_mouse_position(&mut self, pos: Vec2) {
        self.mouse_delta += pos - self.mouse_pos;
        self.mouse_pos = pos;
    }

    pub fn add_wheel(&mut self, lines: f32) {
        self.wheel += lines;
    }

    /// Releases everything held, e.g. when the window loses focus.
    pub fn release_all(&mut self) {
        let keys: Vec<_> = self.down.drain().collect();
        self.released.extend(keys);
        let buttons: Vec<_> = self.mouse_down.drain().collect();
        self.mouse_released.extend(buttons);
    }

    /// Clears the per-frame edges and deltas. Called by the app loop after each frame.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.mouse_pressed.clear();
        self.mouse_released.clear();
        self.mouse_delta = Vec2::ZERO;
        self.wheel = 0.0;
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
    fn mouse_delta_accumulates() {
        let mut i = Input::default();
        i.set_mouse_position(Vec2::new(10.0, 10.0));
        i.end_frame();
        i.set_mouse_position(Vec2::new(15.0, 12.0));
        i.set_mouse_position(Vec2::new(20.0, 10.0));
        assert_eq!(i.mouse_delta(), Vec2::new(10.0, 0.0));
    }
}
