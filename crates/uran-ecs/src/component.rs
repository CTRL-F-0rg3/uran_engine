//! Komponenty renderowania 2D.
//!
//! Encja jest widoczna, jeśli ma komponenty:
//! * `(&Transform, &Sprite, &Material, &Visibility)` — prostokąt z tekstury,
//! * `(&Transform, &Mesh, &Material, &Visibility)` — dowolna siatka trójkątów.

use bytemuck::{Pod, Zeroable};
use uran_asset::{Handle, Image};
use uran_math::{Color, Mat3, Vec2};

/// Transform 2D. `translation` to **środek** obiektu, `rotation` w radianach.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Transform {
    pub translation: Vec2,
    pub rotation: f32,
    pub scale: Vec2,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec2::ZERO,
        rotation: 0.0,
        scale: Vec2::ONE,
    };

    pub fn from_translation(translation: Vec2) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    pub fn from_xy(x: f32, y: f32) -> Self {
        Self::from_translation(Vec2::new(x, y))
    }

    pub fn with_translation(mut self, translation: Vec2) -> Self {
        self.translation = translation;
        self
    }

    pub fn with_rotation(mut self, rotation: f32) -> Self {
        self.rotation = rotation;
        self
    }

    /// Przyjmuje obrót w **stopniach** (wygodniejsze w grach).
    pub fn with_rotation_degrees(self, degrees: f32) -> Self {
        self.with_rotation(degrees.to_radians())
    }

    pub fn with_scale(mut self, scale: Vec2) -> Self {
        self.scale = scale;
        self
    }

    pub fn with_uniform_scale(self, scale: f32) -> Self {
        self.with_scale(Vec2::splat(scale))
    }

    /// Macierz TRS (kolumnowo-majorowa, zgodna z `glam`).
    pub fn matrix(&self) -> Mat3 {
        Mat3::from_scale_angle_translation(self.scale, self.rotation, self.translation)
    }

    /// Przekształca punkt z przestrzeni lokalnej do świata.
    pub fn transform_point(&self, point: Vec2) -> Vec2 {
        (self.matrix() * point.extend(1.0)).truncate()
    }

    /// Kąt w stopniach (debug/UI).
    pub fn rotation_degrees(&self) -> f32 {
        self.rotation.to_degrees()
    }
}

impl From<Vec2> for Transform {
    fn from(translation: Vec2) -> Self {
        Self::from_translation(translation)
    }
}

/// Wierzchołek siatki 2D (32 bajty, `Pod` dla wgpu).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    pub fn new(position: Vec2, color: Color) -> Self {
        Self {
            position: position.to_array(),
            uv: [0.0, 0.0],
            color: color.to_array(),
        }
    }

    pub fn with_uv(mut self, uv: Vec2) -> Self {
        self.uv = uv.to_array();
        self
    }
}

/// Siatka trójkątów: wierzchołki + indeksy (indeksy oszczędzają pamięć i przepustowość).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self { vertices, indices }
    }

    /// Pojedynczy trójkąt (3 wierzchołki, 3 indeksy).
    pub fn triangle(a: Vec2, b: Vec2, c: Vec2, color: Color) -> Self {
        Self {
            vertices: vec![
                Vertex::new(a, color),
                Vertex::new(b, color),
                Vertex::new(c, color),
            ],
            indices: vec![0, 1, 2],
        }
    }

    /// Prostokąt z podanymi wierzchołkami (kolejność: LL, LD, PD, PG).
    pub fn quad(corners: [Vec2; 4], color: Color) -> Self {
        Self {
            vertices: corners.iter().map(|p| Vertex::new(*p, color)).collect(),
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() || self.vertices.is_empty()
    }

    /// Liczba trójkątów w siatce.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Sprawdza, czy indeksy mieszczą się w zakresie wierzchołków.
    pub fn validate(&self) -> bool {
        let max = self.vertices.len() as u32;
        self.indices.iter().all(|i| *i < max)
    }
}

/// Podzbiór tekstury w znormalizowanych UV (0..1) — np. kafelek w atlasie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for UvRect {
    fn default() -> Self {
        Self::FULL
    }
}

impl UvRect {
    pub const FULL: Self = Self {
        min: Vec2::ZERO,
        max: Vec2::ONE,
    };

    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// Wyznacza UV z prostokąta w pikselach + rozmiaru całego obrazu.
    pub fn from_pixels(rect: uran_math::Rect, image_size: Vec2) -> Self {
        let size = image_size.max(Vec2::splat(1.0));
        Self {
            min: rect.min / size,
            max: rect.max / size,
        }
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Przekształca punkt lokalny (0..1) w teksturze na UV.
    pub fn map(&self, t: Vec2) -> Vec2 {
        self.min + (self.max - self.min) * t
    }

    pub fn flip_x(&self) -> Self {
        Self {
            min: Vec2::new(self.max.x, self.min.y),
            max: Vec2::new(self.min.x, self.max.y),
        }
    }

    pub fn flip_y(&self) -> Self {
        Self {
            min: Vec2::new(self.min.x, self.max.y),
            max: Vec2::new(self.max.x, self.min.y),
        }
    }

    /// Przesuwa UV o wektor (przydatne przy animacji atlasu).
    pub fn offset(&self, offset: Vec2) -> Self {
        Self {
            min: self.min + offset,
            max: self.max + offset,
        }
    }
}

/// Geometria sprite'a: prostokąt wypełniany tekstemurą z komponentu [`Material`].
///
/// Środek prostokąta trafia na `Transform.translation`, a `size` mówi,
/// jak duży ma być w jednostkach świata.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sprite {
    /// `None` = natywna szerokość obrazu.
    pub size: Option<Vec2>,
    /// Który fragment tekstury pokazać.
    pub uv: UvRect,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl Sprite {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sized(size: Vec2) -> Self {
        Self {
            size: Some(size),
            ..Self::default()
        }
    }

    pub fn square(size: f32) -> Self {
        Self::sized(Vec2::splat(size))
    }

    pub fn with_uv(mut self, uv: UvRect) -> Self {
        self.uv = uv;
        self
    }

    pub fn with_flip(mut self, flip_x: bool, flip_y: bool) -> Self {
        self.flip_x = flip_x;
        self.flip_y = flip_y;
        self
    }
}

/// Jak ma być mieszany kolor ze sceną.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum BlendMode {
    /// Zwykłe mieszanie alfa — poprawne dla 95% gier 2D.
    #[default]
    Alpha,
    /// Dodawanie światła (iskry, pociski, neon).
    Additive,
    /// Kolory już z pomnożoną alfą.
    Premultiplied,
    /// Bez mieszania — nadpisuje tło.
    Replace,
}

/// Materiał: tint, tekstura, tryb mieszania i warstwa sortowania.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    /// Mnożenie koloru (biały = bez zmian).
    pub color: Color,
    pub texture: Option<Handle<Image>>,
    pub blend: BlendMode,
    /// Warstwa sortowania (rosnąco); równe warstwy rysują się w kolejności
    /// dodania, co pozwala grupować wywołania w batche.
    pub layer: i32,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            color: Color::WHITE,
            texture: None,
            blend: BlendMode::default(),
            layer: 0,
        }
    }
}

impl Material {
    pub fn color(color: Color) -> Self {
        Self {
            color,
            ..Self::default()
        }
    }

    pub fn texture(texture: Handle<Image>) -> Self {
        Self {
            texture: Some(texture),
            ..Self::default()
        }
    }

    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }

    pub fn with_blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
    }
}

/// Czy encja jest widoczna. Domyślnie tak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility(pub bool);

impl Default for Visibility {
    fn default() -> Self {
        Self(true)
    }
}

impl Visibility {
    pub fn is_visible(&self) -> bool {
        self.0
    }

    pub fn hide(&mut self) {
        self.0 = false;
    }

    pub fn show(&mut self) {
        self.0 = true;
    }

    pub fn toggle(&mut self) {
        self.0 = !self.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_layout_is_pod() {
        // Musi być zgodne z VertexBufferLayout w rendererze.
        assert_eq!(std::mem::size_of::<Vertex>(), 32);
        assert_eq!(std::mem::align_of::<Vertex>(), 4);
    }

    #[test]
    fn transform_matrix() {
        let t = Transform::from_xy(10.0, 20.0);
        assert_eq!(t.transform_point(Vec2::ZERO), Vec2::new(10.0, 20.0));

        let rotated = Transform::IDENTITY.with_rotation_degrees(90.0);
        let p = rotated.transform_point(Vec2::new(1.0, 0.0));
        assert!(p.x.abs() < 1e-5 && (p.y - 1.0).abs() < 1e-5);

        let scaled = Transform::IDENTITY.with_uniform_scale(3.0);
        assert_eq!(
            scaled.transform_point(Vec2::new(2.0, 1.0)),
            Vec2::new(6.0, 3.0)
        );
    }

    #[test]
    fn mesh_helpers() {
        let m = Mesh::triangle(Vec2::ZERO, Vec2::X, Vec2::Y, Color::WHITE);
        assert!(!m.is_empty());
        assert_eq!(m.triangle_count(), 1);
        assert!(m.validate());
        assert_eq!(m.vertices.len(), 3);

        let mut bad = m.clone();
        bad.indices = vec![0, 1, 9];
        assert!(!bad.validate());
    }

    #[test]
    fn uv_rect() {
        let uv = UvRect::from_pixels(
            uran_math::Rect::from_xywh(0.0, 0.0, 32.0, 16.0),
            Vec2::new(64.0, 32.0),
        );
        assert_eq!(uv.min, Vec2::ZERO);
        assert_eq!(uv.max, Vec2::new(0.5, 0.5));
        assert_eq!(uv.map(Vec2::new(0.5, 0.5)), Vec2::new(0.25, 0.25));
        assert_eq!(UvRect::FULL.flip_x().min.x, 1.0);
        assert_eq!(UvRect::FULL.flip_y().max.y, 0.0);
    }

    #[test]
    fn visibility_defaults_to_true() {
        let mut v = Visibility::default();
        assert!(v.is_visible());
        v.toggle();
        assert!(!v.is_visible());
        v.show();
        assert!(v.is_visible());
    }
}
