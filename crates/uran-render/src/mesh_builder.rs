use uran_ecs::{Mesh, Material, Visibility, Vertex};
use uran_math::{Color, Vec3};

pub struct MeshBuilder {
    mesh: Mesh,
    material: Material,
    visibility: Visibility,
}

impl MeshBuilder {
    /// Tworzy podstawowy trójkąt w podanych współrzędnych
    pub fn triangle(x: f32, y: f32, z: f32, size: f32) -> Self {
        let vertices = vec![
            Vertex { position: [x, y + size, z], color: [1.0, 0.0, 0.0, 1.0] }, // Górny (czerwony)
            Vertex { position: [x - size, y - size, z], color: [0.0, 1.0, 0.0, 1.0] }, // Lewy (zielony)
            Vertex { position: [x + size, y - size, z], color: [0.0, 0.0, 1.0, 1.0] }, // Prawy (niebieski)
        ];

        Self {
            mesh: Mesh { vertices },
            material: Material {
                color: Color::WHITE,
                texture_path: None,
                is_shaded: false,
            },
            visibility: Visibility(true),
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.material.color = color;
        // Nadpisz kolory wierzchołków na jednolity kolor (uproszczenie)
        for v in &mut self.mesh.vertices {
            v.color = [color.r, color.g, color.b, color.a];
        }
        self
    }

    pub fn texture(mut self, path: &str) -> Self {
        self.material.texture_path = Some(path.to_string());
        self
    }

    pub fn visibility(mut self, visible: bool) -> Self {
        self.visibility = Visibility(visible);
        self
    }

    pub fn is_shaded(mut self, shaded: bool) -> Self {
        self.material.is_shaded = shaded;
        self
    }

    /// Zwraca gotowe komponenty do wstrzyknięcia do ECS
    pub fn build(self) -> (Mesh, Material, Visibility) {
        (self.mesh, self.material, self.visibility)
    }
}