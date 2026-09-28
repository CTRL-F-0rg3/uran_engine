//! Fluent API do budowania siatek — wygodne do prototypowania i narzędzi.
//!
//! ```ignore
//! let mesh = MeshBuilder::quad(Rect::from_xywh(0.0, 0.0, 32.0, 32.0))
//!     .color(Color::from_hex(0xFF8800))
//!     .build();
//! ```

use uran_ecs::Mesh;
use uran_math::{Color, Rect, Vec2};

use crate::shape;

/// Buduje [`Mesh`] krok po kroku.
#[derive(Debug, Clone)]
pub struct MeshBuilder {
    mesh: Mesh,
    color: Color,
}

impl MeshBuilder {
    fn new(mesh: Mesh) -> Self {
        Self { mesh, color: Color::WHITE }
    }

    /// Prostokąt (anchor: lewy dolny róg).
    pub fn quad(rect: Rect) -> Self {
        Self::new(Mesh::quad(rect.corners(), Color::WHITE))
    }

    /// Kwadrat o boku `size` wyśrodkowany w podanym punkcie.
    pub fn square(center: Vec2, size: f32) -> Self {
        Self::quad(Rect::from_center(center, Vec2::splat(size)))
    }

    /// Trójkąt o podanych wierzchołkach.
    pub fn triangle(a: Vec2, b: Vec2, c: Vec2) -> Self {
        Self::new(Mesh::triangle(a, b, c, Color::WHITE))
    }

    /// Okrąg wypełniony.
    pub fn circle(center: Vec2, radius: f32, segments: u32) -> Self {
        Self::new(shape::circle(center, radius, segments, Color::WHITE))
    }

    /// Prostokąt z zaokrąglonymi rogami.
    pub fn rounded_rect(rect: Rect, radius: f32) -> Self {
        Self::new(shape::rounded_rect(rect, radius, 4, Color::WHITE))
    }

    /// Wielokąt (triangulowany automatycznie).
    pub fn polygon(points: &[Vec2]) -> Self {
        Self::new(shape::polygon(points, Color::WHITE))
    }

    /// Odcinek o grubości.
    pub fn line(from: Vec2, to: Vec2, width: f32) -> Self {
        Self::new(shape::thick_line(from, to, width, Color::WHITE))
    }

    /// Z gotowej siatki (np. z ECS).
    pub fn from_mesh(mesh: Mesh) -> Self {
        Self::new(mesh)
    }

    /// Ustawia kolor wszystkich wierzchołków.
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        for vertex in &mut self.mesh.vertices {
            vertex.color = color.to_array();
        }
        self
    }

    /// Kolor wierzchołków narastająco (gradient wzdłuż indeksów).
    pub fn gradient(mut self, from: Color, to: Color) -> Self {
        let count = self.mesh.vertices.len();
        for (i, vertex) in self.mesh.vertices.iter_mut().enumerate() {
            let t = if count <= 1 { 0.0 } else { i as f32 / (count - 1) as f32 };
            vertex.color = from.lerp(to, t).to_array();
        }
        self
    }

    /// Przesuwa całą siatkę.
    pub fn translated(mut self, offset: Vec2) -> Self {
        for vertex in &mut self.mesh.vertices {
            vertex.position[0] += offset.x;
            vertex.position[1] += offset.y;
        }
        self
    }

    /// Obraca siatkę wokół środka jej prostokąta ograniczającego.
    pub fn rotated(mut self, radians: f32) -> Self {
        let rect = self.bounds();
        let center = rect.center();
        let (sin, cos) = radians.sin_cos();
        for vertex in &mut self.mesh.vertices {
            let p = Vec2::new(vertex.position[0], vertex.position[1]) - center;
            vertex.position[0] = (p.x * cos - p.y * sin) + center.x;
            vertex.position[1] = (p.x * sin + p.y * cos) + center.y;
        }
        self
    }

    /// Prostokąt ograniczający siatkę.
    pub fn bounds(&self) -> Rect {
        let mut iter = self.mesh.vertices.iter().map(|v| Vec2::new(v.position[0], v.position[1]));
        let Some(first) = iter.next() else { return Rect::ZERO };
        let mut min = first;
        let mut max = first;
        for p in iter {
            min = min.min(p);
            max = max.max(p);
        }
        Rect::new(min, max)
    }

    /// Zwraca gotową siatkę.
    pub fn build(self) -> Mesh {
        self.mesh
    }

    /// Budowa z poprawieniem koloru (przydatne po `from_mesh`).
    pub fn build_colored(mut self, color: Color) -> Mesh {
        self.color = color;
        self.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_has_two_triangles() {
        let mesh = MeshBuilder::quad(Rect::from_xywh(0.0, 0.0, 10.0, 10.0)).build();
        assert_eq!(mesh.triangle_count(), 2);
        assert!(mesh.validate());
        assert_eq!(mesh.vertices.len(), 4);
    }

    #[test]
    fn color_applies_to_all_vertices() {
        let mesh = MeshBuilder::quad(Rect::from_xywh(0.0, 0.0, 4.0, 4.0))
            .color(Color::RED)
            .build();
        assert!(mesh.vertices.iter().all(|v| v.color == Color::RED.to_array()));
    }

    #[test]
    fn gradient_varies_colors() {
        let mesh = MeshBuilder::quad(Rect::from_xywh(0.0, 0.0, 4.0, 4.0))
            .gradient(Color::BLACK, Color::WHITE)
            .build();
        let first = mesh.vertices[0].color;
        let last = mesh.vertices[3].color;
        assert!(first[0] < last[0]);
    }

    #[test]
    fn translation_moves_bounds() {
        let mesh = MeshBuilder::quad(Rect::from_xywh(0.0, 0.0, 10.0, 10.0))
            .translated(Vec2::new(5.0, 5.0))
            .build();
        assert_eq!(mesh.vertices[0].position, [5.0, 5.0]);
    }

    #[test]
    fn rotation_preserves_size() {
        let rect = Rect::from_xywh(0.0, 0.0, 10.0, 20.0);
        let mesh = MeshBuilder::quad(rect).rotated(std::f32::consts::FRAC_PI_4).build();
        let bounds = MeshBuilder::from_mesh(mesh.clone()).bounds();
        // środek zostaje na miejscu...
        assert!((bounds.center() - rect.center()).length() < 1e-4);
        // ...a realne pole siatki (nie AABB!) jest niezmienne
        let total: f32 = mesh
            .indices
            .chunks(3)
            .map(|t| {
                let p = |i: u32| uran_math::Vec2::from_array(mesh.vertices[i as usize].position);
                (p(t[1]) - p(t[0])).perp_dot(p(t[2]) - p(t[0])).abs() * 0.5
            })
            .sum();
        assert!((total - 200.0).abs() < 0.1, "pole {total} != 200");
    }

    #[test]
    fn bounds_of_empty_mesh_is_zero() {
        assert_eq!(MeshBuilder::from_mesh(Mesh::default()).bounds(), Rect::ZERO);
    }
}
