use glam::{Mat4, Vec2, Vec3};

/// A 2D camera: the world point at the centre of the screen, a zoom and a rotation.
///
/// World coordinates are y-down like screen pixels (and like Godot 2D), so at zoom 1 one world
/// unit is one window pixel. Keep the camera near the origin and move the world instead (a
/// floating origin) when the world is huge, as Cosmic Commando does, so `f32` stays precise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2D {
    pub position: Vec2,
    /// 2.0 = everything twice as big.
    pub zoom: f32,
    /// Radians, clockwise on screen.
    pub rotation: f32,
    /// Window size in pixels.
    pub viewport: Vec2,
}

impl Camera2D {
    pub fn new(viewport: Vec2) -> Self {
        Self { position: Vec2::ZERO, zoom: 1.0, rotation: 0.0, viewport }
    }

    /// World to clip space.
    pub fn view_proj(&self) -> Mat4 {
        let half = self.viewport * 0.5 / self.zoom.max(1e-6);
        // y-down world, y-up clip space: flip y. Everything sits at depth 0.
        let proj = Mat4::from_scale(Vec3::new(1.0 / half.x, -1.0 / half.y, 1.0));
        let view =
            Mat4::from_rotation_z(-self.rotation) * Mat4::from_translation((-self.position).extend(0.0));
        proj * view
    }

    /// Window pixel (top-left origin) to world position.
    pub fn screen_to_world(&self, screen: Vec2) -> Vec2 {
        let centred = (screen - self.viewport * 0.5) / self.zoom.max(1e-6);
        Vec2::from_angle(self.rotation).rotate(centred) + self.position
    }

    /// World position to window pixel (top-left origin).
    pub fn world_to_screen(&self, world: Vec2) -> Vec2 {
        let rel = Vec2::from_angle(-self.rotation).rotate(world - self.position);
        rel * self.zoom + self.viewport * 0.5
    }

    /// The world-space size of the view (ignoring rotation).
    pub fn visible_size(&self) -> Vec2 {
        self.viewport / self.zoom.max(1e-6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_world_round_trip() {
        let mut c = Camera2D::new(Vec2::new(1920.0, 1080.0));
        c.position = Vec2::new(500.0, -200.0);
        c.zoom = 2.5;
        c.rotation = 0.7;
        for p in [Vec2::ZERO, Vec2::new(100.0, 900.0), Vec2::new(1920.0, 1080.0)] {
            let back = c.world_to_screen(c.screen_to_world(p));
            assert!((back - p).length() < 1e-2, "{p} -> {back}");
        }
    }

    #[test]
    fn view_proj_maps_screen_corners() {
        let mut c = Camera2D::new(Vec2::new(800.0, 600.0));
        c.position = Vec2::new(10.0, 20.0);
        c.zoom = 2.0;
        let vp = c.view_proj();
        let top_left = c.screen_to_world(Vec2::ZERO);
        let clip = vp.project_point3(top_left.extend(0.0));
        assert!((clip.x + 1.0).abs() < 1e-5 && (clip.y - 1.0).abs() < 1e-5, "{clip}");
        let centre = vp.project_point3(c.position.extend(0.0));
        assert!(centre.truncate().length() < 1e-5);
    }
}
