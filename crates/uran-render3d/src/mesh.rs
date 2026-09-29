//! Geometria 3D: wierzchołki po stronie CPU i ich kopia na GPU.

use bytemuck::{Pod, Zeroable};
// `Vec4` uzywaja wylacznie testy (przeksztalcanie punktow przez
// macierz), wiec trzymamy go pod `cfg(test)` — bez tego kompilacja
// produkcyjna zglasza nieuzywany import.
#[cfg(test)]
use uran_math::Vec4;
use uran_math::{Mat3, Mat4, Quat, Vec2, Vec3};
use wgpu::util::DeviceExt;

/// Wierzchołek siatki: pozycja, normalna, UV, kolor.
///
/// ## Dlaczego UV jest tutaj, a nie osobny bufor
///
/// Tekstury wczytujemy z plików modeli (`.obj` z UV z Blendera), więc
/// UV musi docierać do shadera dla KAŻDEJ siatki. Trzymanie go w tym
/// samym `Vertex` oznacza jeden bufor wierzchołków i brak dodatkowych
/// powiązań; cena to 8 B na wierzchołek, co przy 30 tys. wierzchołków
/// motocykla daje ok. 240 kB — mniej niż jedna tekstura 1k.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Współrzędne tekstury 0..1.
    pub uv: [f32; 2],
    pub color: [f32; 3],
}

impl Vertex {
    /// Wierzchołek o pozycji i kolorze, bez tekstury.
    ///
    /// Normalną trzeba ustawić świadomie — „normalna = pozycja" daje
    /// złe oświetlenie i cicho psuje wygląd bryły. UV dostaje (0,0),
    /// czyli brak tekstury: shader i tak użyje koloru wierzchołka.
    pub fn new(position: Vec3, normal: Vec3, color: [f32; 3]) -> Self {
        Self {
            position: position.to_array(),
            normal: normal.to_array(),
            uv: [0.0, 0.0],
            color,
        }
    }

    /// Wierzchołek z teksturą — używane przez importer modeli.
    pub fn new_uv(position: Vec3, normal: Vec3, uv: [f32; 2], color: [f32; 3]) -> Self {
        Self {
            position: position.to_array(),
            normal: normal.to_array(),
            uv,
            color,
        }
    }

    pub fn pos(&self) -> Vec3 {
        Vec3::from_array(self.position)
    }

    pub fn normal(&self) -> Vec3 {
        Vec3::from_array(self.normal)
    }

    /// Czy wierzchołek ma niezerowe UV.
    ///
    /// Używane przy decyzji, czy siatka w ogóle potrzebuje tekstury —
    /// geometria budowana w kodzie (tanki, droga) nie ma.
    pub fn has_uv(&self) -> bool {
        self.uv != [0.0, 0.0]
    }
}

/// Siatka trzymana po stronie CPU (przed wgraniem na kartę).
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    /// Indeksy trójkątów. Pusta lista = tryb `draw` zamiast `draw_indexed`.
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dodaje wierzchołek i zwraca jego indeks.
    pub fn push(&mut self, v: Vertex) -> u32 {
        self.vertices.push(v);
        (self.vertices.len() - 1) as u32
    }

    /// Dodaje trójkąt trzema indeksami.
    pub fn triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// Dodaje wierzchołek i trójkąt z nim w jednym wywołaniu.
    pub fn tri(&mut self, a: Vertex, b: Vertex, c: Vertex) {
        let i = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&[a, b, c]);
        self.indices.extend_from_slice(&[i, i + 1, i + 2]);
    }

    /// Czy bryła ma komplet trójkątów.
    pub fn is_closed(&self) -> bool {
        self.indices.len() % 3 == 0 && !self.indices.is_empty()
    }

    /// Środek bryły średnią wierzchołków.
    ///
    /// Używamy średniej, nie AABB — dla symetrycznych brył (czołg, blok)
    /// to się pokrywa, a dla skrzydeł daje punkt wewnątrz kadłuba.
    pub fn center(&self) -> Vec3 {
        if self.vertices.is_empty() {
            return Vec3::ZERO;
        }
        let sum: Vec3 = self.vertices.iter().map(|v| v.pos()).sum();
        sum / self.vertices.len() as f32
    }

    /// Wgrywa siatkę na kartę.
    pub fn upload(&self, device: &wgpu::Device, label: &str) -> GpuMesh {
        GpuMesh::new(device, label, self)
    }
}

/// Siatka trzymana na GPU.
pub struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: Option<wgpu::Buffer>,
    /// `0` = rysuj `draw`, `>0` = rysuj `draw_indexed`.
    index_count: u32,
    vertex_count: u32,
}

impl GpuMesh {
    pub fn new(device: &wgpu::Device, label: &str, mesh: &Mesh) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} (wierzchołki)")),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Pusty bufor indeksów jest błędem walidacji wgpu, więc trzymamy
        // `Option` i rysujemy albo indeksowo, albo sekwencyjnie.
        let (index_buffer, index_count) = if mesh.indices.is_empty() {
            (None, 0)
        } else {
            (
                Some(
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("{label} (indeksy)")),
                        contents: bytemuck::cast_slice(&mesh.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                ),
                mesh.indices.len() as u32,
            )
        };

        Self {
            vertex_buffer,
            index_buffer,
            index_count,
            vertex_count: mesh.vertices.len() as u32,
        }
    }

    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertex_buffer
    }

    /// Ustawia slot wierzchołków i rysuje.
    ///
    /// `base_instance` musi być ≥ 1, bo WGSL liczy `instance_index` od 1
    /// przy `first_instance`. Dzięki temu każdy obiekt sięga po własną
    /// macierz modelu we WSPÓLNYM buforze, zamiast mieć osobny bufor.
    /// `&'r self` (a nie `&self`) jest tu konieczne: `set_index_buffer`
    /// wymaga `BufferSlice<'r>`, więc slice musi żyć tak długo jak pass.
    /// Stąd też pożyczka buforów jest na czas rysowania, a nie na czas
    /// całej klatki.
    pub fn draw<'r>(&'r self, pass: &mut wgpu::RenderPass<'r>, slot: u32, base_instance: u32) {
        pass.set_vertex_buffer(slot, self.vertex_buffer.slice(..));
        match &self.index_buffer {
            Some(ib) => {
                pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, base_instance..base_instance + 1);
            }
            // brak indeksów: każda trójka wierzchołków to trójkąt
            None => pass.draw(0..self.vertex_count, base_instance..base_instance + 1),
        }
    }
}

/// Uniform przekazywany do shadera 3D w jednej klatce.
///
/// UKŁAD MUSI BYĆ ZGODNY Z `Scene` W `s3d.wgsl`. Wszystkie pola
/// trzymamy w `vec4` także po stronie Rusta, bo WGSL wyrównuje `vec3`
/// do 16 B — struktura z samymi `vec3` urosłaby i walidacja wgpu
/// odrzuciłaby bufor.
///
/// ## Rozmiar: 224 B
///
/// Liczymy pole po polu, bo to jednocześnie kontrakt z shaderem
/// i asercja kompilacji:
///
/// | pole | rozmiar |
/// |------|---------|
/// | `view_proj` | 4·`vec4` = 64 B |
/// | `eye`, `light_dir`, `light_color`, `ambient_time` | 4·16 = 64 B |
/// | `light_view_proj` | 4·`vec4` = 64 B |
/// | `shadow_params`, `shadow_map_info` | 2·16 = 32 B |
/// | **suma** | **224 B** |
///
/// Wszystko jest 16-bajtowo wyrównane, więc `size % 16 == 0` i wgpu
/// nie zgłosi błędu wyrównania.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SceneUniform {
    /// Macierz świata -> NDC.
    pub view_proj: [[f32; 4]; 4],
    /// Pozycja oka (do specular), `w` nieużywane.
    pub eye: [f32; 4],
    /// Kierunek światła kierunkowego (normalizowany w shaderze).
    pub light_dir: [f32; 4],
    /// Kolor światła.
    pub light_color: [f32; 4],
    /// `rgb` = ambient, `a` = czas świata.
    pub ambient_time: [f32; 4],

    /// --- mapowanie cieni ---
    ///
    /// Świat -> NDC tekstury cieni. Osobna macierz, bo kamera cienia
    /// stoi w zupełnie innym miejscu niż oko gracza: patrzy z pozycji
    /// słońca, ortograficznie, na całą scenę.
    pub light_view_proj: [[f32; 4]; 4],
    /// `x` = bias w głębokości, `y` = odsunięcie wzdłuż normalnej,
    /// `z` = siła cienia, `w` = promień PCF (w texelach).
    pub shadow_params: [f32; 4],
    /// `x` = rozmiar texela w UV, `y` = włącznik (0/1),
    /// `z`,`w` = wyrównanie na 16 B.
    pub shadow_map_info: [f32; 4],
}

/// Rozmiar uniformu jest częścią kontraktu z shaderem: zmiana jednego bez
/// drugiego kończy się błędem walidacji wgpu dopiero podczas renderowania.
const _: () = assert!(std::mem::size_of::<SceneUniform>() == 224);

/// Transformacja i kolor jednego obiektu w scenie.
///
/// Zamiast osobnego bufora per obiekt (kilkanaście `write_buffer` na
/// klatkę przy kilkunastu czołgach) trzymamy JEDNĄ tablicę i sięgamy po
/// niej indeksem instancji w shaderze.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct InstanceModel {
    /// Macierz modelu (kolumnowa).
    pub model: [[f32; 4]; 4],
    /// Kolor podstawowy — mnożymy nim kolor wierzchołka, żeby odróżnić
    /// drużyny bez osobnych siatek i bez tekstur.
    pub tint: [f32; 4],
}

/// Macierz modelu: skalowanie -> obrót -> przesunięcie.
///
/// Kolejność `T * R * S` wynika z kolumnowego zapisu macierzy; odwrotna
/// kolejność przesunęłaby obiekt o złą skalę.
///
/// Obrót budujemy jako iloczyn kwaternionów w kolejności Y-X-Z:
/// najpierw yaw (obrót wokół pionu), potem pitch (pochylenie lufy),
/// na końcu roll (przechył na boki). Inna kolejność daje „zaplątany"
/// obrót, którego nie da się opisać trzema kątami.
pub fn model_matrix(position: Vec3, yaw: f32, pitch: f32, roll: f32, scale: Vec3) -> Mat4 {
    let rotation =
        Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch) * Quat::from_rotation_z(roll);
    Mat4::from_translation(position)
        * Mat4::from_mat3(Mat3::from_quat(rotation))
        * Mat4::from_scale(scale)
}

/// Osiem narożników prostopadłościanu (kolejność: 0-3 spód, 4-7 góra).
pub fn box_corners(center: Vec3, half: Vec3) -> [Vec3; 8] {
    [
        center + Vec3::new(-half.x, -half.y, -half.z),
        center + Vec3::new(half.x, -half.y, -half.z),
        center + Vec3::new(half.x, -half.y, half.z),
        center + Vec3::new(-half.x, -half.y, half.z),
        center + Vec3::new(-half.x, half.y, -half.z),
        center + Vec3::new(half.x, half.y, -half.z),
        center + Vec3::new(half.x, half.y, half.z),
        center + Vec3::new(-half.x, half.y, half.z),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri_mesh() -> Mesh {
        let mut m = Mesh::new();
        m.tri(
            Vertex::new(Vec3::ZERO, Vec3::Z, [1.0, 0.0, 0.0]),
            Vertex::new(Vec3::X, Vec3::Z, [1.0, 0.0, 0.0]),
            Vertex::new(Vec3::Y, Vec3::Z, [1.0, 0.0, 0.0]),
        );
        m
    }

    #[test]
    fn vertex_is_44_bytes_and_uniform_is_aligned() {
        // 3 pozycja + 3 normalna + 2 UV + 3 kolor = 11 * 4 B = 44 B.
        // Rozmiar jest częścią kontraktu z `VertexBufferLayout` w potoku —
        // zmiana jednego bez drugiego psuje renderowanie po cichu.
        assert_eq!(std::mem::size_of::<Vertex>(), 44);
        assert_eq!(std::mem::size_of::<InstanceModel>(), 80);
        assert_eq!(std::mem::size_of::<SceneUniform>() % 16, 0);
    }

    #[test]
    fn tri_adds_three_indices() {
        let m = tri_mesh();
        assert_eq!(m.vertices.len(), 3);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert!(m.is_closed());
    }

    #[test]
    fn empty_mesh_is_not_closed() {
        assert!(!Mesh::new().is_closed());
    }

    #[test]
    fn center_averages_vertices() {
        let mut m = Mesh::new();
        m.push(Vertex::new(Vec3::ZERO, Vec3::Z, [0.0; 3]));
        m.push(Vertex::new(Vec3::new(2.0, 0.0, 0.0), Vec3::Z, [0.0; 3]));
        let c = m.center();
        assert!((c.x - 1.0).abs() < 1e-6, "środek to {c:?}");
    }

    #[test]
    fn model_matrix_applies_scale_before_translation() {
        // punkt (1,0,0) przeskalowany 2x i przesunięty o (5,0,0) -> (7,0,0)
        let m = model_matrix(Vec3::new(5.0, 0.0, 0.0), 0.0, 0.0, 0.0, Vec3::splat(2.0));
        let p = m * Vec4::new(1.0, 0.0, 0.0, 1.0);
        assert!((p.x - 7.0).abs() < 1e-5, "otrzymano {p:?} zamiast (7,0,0)");
    }

    #[test]
    fn model_matrix_keeps_centre_in_place_when_rotating() {
        let m = model_matrix(
            Vec3::new(3.0, 0.0, 0.0),
            std::f32::consts::FRAC_PI_2,
            0.0,
            0.0,
            Vec3::ONE,
        );
        let centre = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!((centre.x - 3.0).abs() < 1e-5, "środek uciekł: {centre:?}");
    }

    #[test]
    fn box_corners_are_eight_distinct_points() {
        let c = box_corners(Vec3::new(1.0, 2.0, 3.0), Vec3::splat(0.5));
        assert_eq!(c.len(), 8);
        for (i, a) in c.iter().enumerate() {
            for (j, b) in c.iter().enumerate() {
                if i != j {
                    assert!(
                        (*a - *b).length() > 1e-6,
                        "narożniki {i} i {j} pokrywają się"
                    );
                }
            }
        }
    }
}

/// Punkt na trójkącie `a b c` dla współrzędnych barycentrycznych `t`.
pub fn barycentric(a: Vec3, b: Vec3, c: Vec3, t: Vec2) -> Vec3 {
    a * (1.0 - t.x - t.y) + b * t.x + c * t.y
}
