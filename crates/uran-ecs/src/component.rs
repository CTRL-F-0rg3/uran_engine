use uran_math::{Color, Vec3};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: f32,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self { translation: Vec3::ZERO, rotation: 0.0, scale: Vec3::ONE }
    }
}

// --- NOWE KOMPONENTY ---

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

// Implementacja wymagana przez wgpu do przesyłania danych do GPU
unsafe impl bytemuck::Pod for Vertex {}
unsafe impl bytemuck::Zeroable for Vertex {}

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub color: Color,
    pub texture_path: Option<String>, // Na razie jako String, później załadujemy to jako Handle
    pub is_shaded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visibility(pub bool);