use uran_math::Vec3; // Zakładamy, że dodasz glam do uran-math, na razie używamy placeholdera lub zdefiniujmy prosty typ

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: f32, // W radianach, wokół osi Z (dla 2D)
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::new(0.0, 0.0, 0.0),
            rotation: 0.0,
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }
}

// Marker component, np. dla przyszłych obrazków
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sprite; 