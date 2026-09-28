//! Lista rysowania i batching: zamienia wywołania gry w optymalną kolejność
//! draw calli. Cała logika jest czysta i testowalna bez GPU.

use bytemuck::{Pod, Zeroable};
use uran_asset::{Handle, HandleId, Image};
use uran_ecs::{BlendMode, Material, Mesh, Sprite, Transform, Visibility};
use uran_math::{Color, Mat3, Vec2};
use uran_ecs::UvRect;
use uran_ecs::World;

/// Dane jednej instancji sprite'a przesyłane do GPU (64 bajty).
///
/// Kolejność pól odpowiada atrybutom w `sprite.wgsl` (lokalizacje 1..4).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SpriteInstance {
    /// (m00, m01, m10, m11) — liniowa część macierzy 2x3.
    pub m0: [f32; 4],
    /// (m20, m21, 0, 0) — translacja.
    pub m1: [f32; 4],
    /// (u0, v0, u1, v1) — fragment tekstury.
    pub uv: [f32; 4],
    /// Tint RGBA w **liniowym** świetle.
    pub color: [f32; 4],
}

impl SpriteInstance {
    /// Buduje instancję z macierzy świata (zawiera już skalę rozmiaru).
    pub fn new(transform: Mat3, uv: UvRect, color: Color) -> Self {
        let c = transform.to_cols_array_2d();
        Self {
            m0: [c[0][0], c[0][1], c[1][0], c[1][1]],
            m1: [c[2][0], c[2][1], 0.0, 0.0],
            uv: [uv.min.x, uv.min.y, uv.max.x, uv.max.y],
            color: color.to_linear().to_array(),
        }
    }
}

/// Dane per-draw dla potoku siatek (push constants, 64 bajty).
///
/// `mat3x3<f32>` w WGSL zajmuje 3 kolumny po 16 bajtów = 48 bajtów.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MeshPushConstants {
    /// 3 kolumny macierzy 3x3 (kolumnowo-majorowa).
    pub model: [[f32; 4]; 3],
    pub tint: [f32; 4],
}

impl MeshPushConstants {
    pub fn new(transform: Mat3, color: Color) -> Self {
        let c = transform.to_cols_array_2d();
        let tint = color.to_linear();
        Self {
            model: [
                [c[0][0], c[0][1], c[0][2], 0.0],
                [c[1][0], c[1][1], c[1][2], 0.0],
                [c[2][0], c[2][1], c[2][2], 0.0],
            ],
            tint: tint.to_array(),
        }
    }
}

/// Globalna macierz świata -> NDC (group 1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
}

impl Globals {
    /// Homogenizuje macierz 2x3 do macierzy 4x4 w formacie kolumnowym
    /// (takiego, jakiego oczekuje WGSL).
    ///
    /// Dwie pułapki, obie kosztujące „ciche" błędy renderowania:
    /// 1. w `glam` `to_cols_array_2d()[i]` to **kolumna** `i`, a translacja
    ///    leży w ostatniej kolumnie (`c[2]`), a nie w ostatnim elemencie
    ///    każdej kolumny,
    /// 2. wiersz `w` (czwarty w każdej kolumnie) MUSI być zerowy poza
    ///    elementem (3,3). Wpisanie tam `c[2][2] == 1.0` daje `w = 2`
    ///    dla wierzchołków o `z = 0`, a dzielenie perspektywiczne w NDC
    ///    skaluje całą scenę do połowy rozmiaru.
    pub fn new(view_proj: Mat3) -> Self {
        let c = view_proj.to_cols_array_2d();
        Self {
            view_proj: [
                [c[0][0], c[0][1], 0.0, 0.0], // oś X
                [c[1][0], c[1][1], 0.0, 0.0], // oś Y
                [c[2][0], c[2][1], 1.0, 0.0], // translacja (wiersz w = 0!)
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Punkt 2D przez tę macierz, z dzieleniem perspektywicznym (do testów).
    ///
    /// Macierz jest kolumnowa, więc element w wierszu `r` i kolumnie `c`
    /// to `view_proj[c][r]`.
    pub fn transform_point(&self, point: Vec2) -> Vec2 {
        let m = &self.view_proj;
        let x = m[0][0] * point.x + m[1][0] * point.y + m[2][0];
        let y = m[0][1] * point.x + m[1][1] * point.y + m[2][1];
        // w = m[0][3]*x + m[1][3]*y + m[2][3] + m[3][3]; dla naszych macierzy
        // zawsze 1, ale dzielimy, żeby test wyłapał regresję
        let w = m[0][3] * point.x + m[1][3] * point.y + m[2][3] + m[3][3];
        if w.abs() < 1e-9 {
            return Vec2::new(f32::NAN, f32::NAN);
        }
        Vec2::new(x / w, y / w)
    }
}

/// Klucz tekstury GPU. Osobne przestrzenie dla obrazów i atlasów glifów,
/// żeby uchwuty z różnych magazynów nie kolidowały.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextureKey {
    /// Tekstura 1x1 — biała (używana, gdy materiał nie ma tekstury).
    White,
    Image(HandleId),
    /// Atlas glifów danej czcionki.
    FontAtlas(HandleId),
}

/// Wyrównanie w pionie tekstu względem linii bazowej.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Polecenie narysowania tekstu (rozkładane na glify podczas renderowania).
#[derive(Debug, Clone, PartialEq)]
pub struct TextDraw {
    pub font: Handle<uran_asset::FontData>,
    pub text: Box<str>,
    /// Pozycja środka pierwszej linii bazowej (w świecie).
    pub position: Vec2,
    /// Wysokość tekstu w jednostkach świata (odpowiada `em`).
    pub size: f32,
    pub color: Color,
    pub layer: i32,
    pub blend: BlendMode,
    pub align: TextAlign,
    /// Mnożnik interlinii (domyślnie 1.0).
    pub line_height: f32,
    /// Opcjonalne łamanie wierszy przy tej szerokości (w jednostkach świata).
    pub max_width: Option<f32>,
    /// Czy rysować w przestrzeni ekranu (HUD) zamiast świata.
    pub screen_space: bool,
}

/// Geometria siatki wraz z haszem treści (do cache'owania na GPU).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshGeometry {
    pub vertices: Vec<uran_ecs::Vertex>,
    pub indices: Vec<u32>,
    /// FNV-1a z bajtów geometrii — klucz cache'a w rendererze.
    pub hash: u64,
}

impl MeshGeometry {
    pub fn from_mesh(mesh: &Mesh) -> Self {
        let hash = hash_mesh(mesh);
        Self { vertices: mesh.vertices.clone(), indices: mesh.indices.clone(), hash }
    }
}

/// FNV-1a nad zawartością geometrii (po bajtach, z obsługą `f32`).
pub fn hash_mesh(mesh: &Mesh) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    // Jeden helper zamiast zagnieżdżonych closure'ów — `eat` i `eat_floats`
    // pożyczałyby `hash` wielokrotnie i nie kompilowały się.
    fn eat(hash: &mut u64, values: &[f32]) {
        for value in values {
            for byte in value.to_bits().to_le_bytes() {
                *hash ^= byte as u64;
                *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    fn eat_u64(hash: &mut u64, value: u64) {
        for byte in value.to_le_bytes() {
            *hash ^= byte as u64;
            *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    eat_u64(&mut hash, mesh.vertices.len() as u64);
    for vertex in &mesh.vertices {
        eat(&mut hash, &vertex.position);
        eat(&mut hash, &vertex.uv);
        eat(&mut hash, &vertex.color);
    }
    eat_u64(&mut hash, mesh.indices.len() as u64);
    for index in &mesh.indices {
        eat_u64(&mut hash, *index as u64);
    }
    hash
}

/// Jedno polecenie narysowania prostokąta (tekstury).
#[derive(Debug, Clone, PartialEq)]
pub struct SpriteDraw {
    /// Macierz świata (zawiera pozycję, obrót i skalę/rozmiar).
    pub transform: Mat3,
    pub uv: UvRect,
    pub color: Color,
    pub texture: TextureKey,
    pub blend: BlendMode,
    pub layer: i32,
    /// Czy rysować w przestrzeni ekranu (HUD) zamiast świata.
    pub screen_space: bool,
}

impl SpriteDraw {
    /// Prostokąt w przestrzeni świata: `rect` w lokalnych współrzędnych
    /// (anchor: `min`), `transform` to macierz ze stosu transformacji.
    pub fn rect(rect: uran_math::Rect, transform: Mat3, uv: UvRect, color: Color) -> Self {
        let matrix =
            transform * Mat3::from_translation(rect.center()) * Mat3::from_scale(rect.size());
        Self {
            transform: matrix,
            uv,
            color,
            texture: TextureKey::White,
            blend: BlendMode::Alpha,
            layer: 0,
            screen_space: false,
        }
    }
}

/// Jedno polecenie narysowania siatki.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshDraw {
    /// Indeks geometrii w arenie [`DrawList::geometry`].
    pub geometry: u32,
    pub transform: Mat3,
    pub color: Color,
    pub texture: TextureKey,
    pub blend: BlendMode,
    pub layer: i32,
    /// Czy rysować w przestrzeni ekranu (HUD) zamiast świata.
    pub screen_space: bool,
}

/// Polecenie narysowania tekstury 9-slice.
///
/// Rozwijane na 9 prostokątów przez renderer (zna rozmiar tekstury, którego
/// nie ma w `DrawList`).
#[derive(Debug, Clone, PartialEq)]
pub struct NineSliceDraw {
    pub texture: Handle<Image>,
    /// Prostokąt docelowy w lokalnych współrzędnych.
    pub dst: uran_math::Rect,
    pub transform: Mat3,
    /// Margines w pikselach obrazu źródłowego.
    pub border: f32,
    pub color: Color,
    pub blend: BlendMode,
    pub layer: i32,
    /// Czy rysować w przestrzeni ekranu (HUD) zamiast świata.
    pub screen_space: bool,
}

/// Lista wszystkiego, co ma zostać narysowane w jednej klatce.
///
/// Bufor jest wielokrotnego użytku — `clear()` zeruje zawartość, ale
/// zachowuje alokacje, więc po pierwszej klatce nie ma już kosztu `malloc`.
#[derive(Debug, Default)]
pub struct DrawList {
    sprites: Vec<SpriteDraw>,
    meshes: Vec<MeshDraw>,
    texts: Vec<TextDraw>,
    nine_slices: Vec<NineSliceDraw>,
    geometry: Vec<MeshGeometry>,
    /// Przygotowane instancje (wypełniane przez renderer).
    pub(crate) instances: Vec<SpriteInstance>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Czyści listę, zachowując alokacje.
    pub fn clear(&mut self) {
        self.sprites.clear();
        self.meshes.clear();
        self.texts.clear();
        self.nine_slices.clear();
        self.geometry.clear();
        self.instances.clear();
    }

    pub fn sprites(&self) -> &[SpriteDraw] {
        &self.sprites
    }

    /// Podglądowy dostęp do sprite'ów.
    pub fn sprites_mut(&mut self) -> &mut Vec<SpriteDraw> {
        &mut self.sprites
    }

    /// Podmienia listę sprite'ów (używane przy rozwijaniu tekstu).
    pub fn replace_sprites(&mut self, sprites: Vec<SpriteDraw>) {
        self.sprites = sprites;
    }

    pub fn meshes(&self) -> &[MeshDraw] {
        &self.meshes
    }

    pub fn meshes_mut(&mut self) -> &mut Vec<MeshDraw> {
        &mut self.meshes
    }

    pub fn texts(&self) -> &[TextDraw] {
        &self.texts
    }

    pub fn clear_texts(&mut self) {
        self.texts.clear();
    }

    pub fn nine_slices(&self) -> &[NineSliceDraw] {
        &self.nine_slices
    }

    pub fn clear_nine_slices(&mut self) {
        self.nine_slices.clear();
    }

    pub fn geometry(&self) -> &[MeshGeometry] {
        &self.geometry
    }

    /// Liczba elementów do narysowania (do statystyk).
    pub fn len(&self) -> usize {
        self.sprites.len() + self.meshes.len() + self.texts.len() + self.nine_slices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push_sprite(&mut self, sprite: SpriteDraw) {
        self.sprites.push(sprite);
    }

    pub fn push_text(&mut self, text: TextDraw) {
        self.texts.push(text);
    }

    pub fn push_nine_slice(&mut self, slice: NineSliceDraw) {
        self.nine_slices.push(slice);
    }

    /// Dodaje siatkę do areny i zwraca jej indeks.
    pub fn push_mesh(&mut self, mesh: &Mesh) -> u32 {
        self.push_mesh_geometry(MeshGeometry::from_mesh(mesh))
    }

    pub fn push_mesh_geometry(&mut self, geometry: MeshGeometry) -> u32 {
        self.geometry.push(geometry);
        (self.geometry.len() - 1) as u32
    }

    pub fn add_mesh_draw(&mut self, geometry: u32, transform: Mat3, color: Color) {
        self.meshes.push(MeshDraw {
            geometry,
            transform,
            color,
            texture: TextureKey::White,
            blend: BlendMode::Alpha,
            layer: 0,
            screen_space: false,
        });
    }

    /// Wprawia encje z ECS do listy rysowania.
    ///
    /// Obsługiwane są dwa zestawy komponentów:
    /// * `(&Transform, &Sprite, &Material, &Visibility)` — sprite,
    /// * `(&Transform, &Mesh, &Material, &Visibility)` — siatka.
    ///
    /// Encja z obydwoma naraz zostanie narysowana dwukrotnie — używaj
    /// jednego z nich.
    pub fn extract_world(&mut self, world: &World, assets: &uran_asset::AssetServer) {
        for (_entity, (transform, sprite, material, visibility)) in
            world.query::<(&Transform, &Sprite, &Material, &Visibility)>().iter()
        {
            if !visibility.0 {
                continue;
            }
            let size = sprite.size.unwrap_or_else(|| {
                material
                    .texture
                    .and_then(|t| assets.image(t))
                    .map(|i| Vec2::new(i.width as f32, i.height as f32))
                    .unwrap_or(Vec2::ONE)
            });
            let mut uv = sprite.uv;
            if sprite.flip_x {
                uv = uv.flip_x();
            }
            if sprite.flip_y {
                uv = uv.flip_y();
            }
            self.sprites.push(SpriteDraw {
                transform: transform.matrix() * Mat3::from_scale(size),
                uv,
                color: material.color,
                texture: material
                    .texture
                    .map(|t| TextureKey::Image(t.into()))
                    .unwrap_or(TextureKey::White),
                blend: material.blend,
                layer: material.layer,
                screen_space: false,
            });
        }

        for (_entity, (transform, mesh, material, visibility)) in
            world.query::<(&Transform, &Mesh, &Material, &Visibility)>().iter()
        {
            if !visibility.0 || mesh.is_empty() || !mesh.validate() {
                continue;
            }
            let geometry = self.push_mesh_geometry(MeshGeometry::from_mesh(mesh));
            self.meshes.push(MeshDraw {
                geometry,
                transform: transform.matrix(),
                color: material.color,
                texture: material
                    .texture
                    .map(|t| TextureKey::Image(t.into()))
                    .unwrap_or(TextureKey::White),
                blend: material.blend,
                layer: material.layer,
                screen_space: false,
            });
        }
    }

    /// Sortuje sprite'y i siatki tak, aby zminimalizować liczbę zmian
    /// pipeline'u i bind groupów, zachowując kolejność malarza w obrębie warstwy.
    ///
    /// Sortowanie jest stabilne: równe klucze zachowują kolejność dodania,
    /// więc rysowanie „od góry" w obrębie jednej warstwy działa poprawnie.
    pub fn sort(&mut self) {
        // `screen_space` jest pierwszym kluczem: HUD rysowany jest ZAWSZE
        // na wierzchu, niezależnie od warstwy, którą dostał w świecie.
        self.sprites.sort_by(|a, b| {
            (a.screen_space, a.layer, a.blend, a.texture)
                .cmp(&(b.screen_space, b.layer, b.blend, b.texture))
        });
        self.meshes.sort_by(|a, b| {
            (a.screen_space, a.layer, a.blend, a.texture)
                .cmp(&(b.screen_space, b.layer, b.blend, b.texture))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uran_math::Rect;

    fn sprite(texture: TextureKey, layer: i32) -> SpriteDraw {
        SpriteDraw {
            transform: Mat3::IDENTITY,
            uv: UvRect::FULL,
            screen_space: false,
            color: Color::WHITE,
            texture,
            blend: BlendMode::Alpha,
            layer,
        }
    }

    #[test]
    fn rect_sprite_centers_and_scales() {
        let draw = SpriteDraw::rect(
            Rect::from_xywh(0.0, 0.0, 10.0, 20.0),
            Mat3::IDENTITY,
            UvRect::FULL,
            Color::WHITE,
        );
        // środek prostokąta (5, 10) trafia na macierz
        assert_eq!((draw.transform * Vec2::ZERO.extend(1.0)).truncate(), Vec2::new(5.0, 10.0));
        // macierz przekształca współrzędne JEDNOSTKOWEGO prostokąta
        // (-0.5..0.5), więc róg (0.5, 0.5) ma trafić w (10, 20)
        let corner = (draw.transform * Vec2::new(0.5, 0.5).extend(1.0)).truncate();
        assert!((corner - Vec2::new(10.0, 20.0)).length() < 1e-5);
    }

    #[test]
    fn instance_layout_matches_wgsl() {
        let m = Mat3::from_translation(Vec2::new(3.0, 4.0));
        let instance = SpriteInstance::new(m, UvRect::new(Vec2::ZERO, Vec2::ONE), Color::WHITE);
        assert_eq!(std::mem::size_of::<SpriteInstance>(), 64);
        assert_eq!(instance.m0, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(instance.m1, [3.0, 4.0, 0.0, 0.0]);
        assert_eq!(instance.uv, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn instance_converts_color_to_linear() {
        let instance =
            SpriteInstance::new(Mat3::IDENTITY, UvRect::FULL, Color::from_hex(0x808080));
        assert!(instance.color[0] < 0.5, "kolor powinien być konwertowany do liniowego");
        assert!((instance.color[0] - 0.2158).abs() < 0.01);
    }

    #[test]
    fn push_constants_size_is_64() {
        // mat3x3<f32> w WGSL = 48 bajtów + vec4 = 64
        assert_eq!(std::mem::size_of::<MeshPushConstants>(), 64);
        let push =
            MeshPushConstants::new(Mat3::from_translation(Vec2::new(1.0, 2.0)), Color::WHITE);
        assert_eq!(push.model[2][0], 1.0);
        assert_eq!(push.model[2][1], 2.0);
        assert_eq!(push.model[0][3], 0.0, "kolumny mat3 w GLSL są wyrównane do 16 bajtów");
        assert_eq!(push.tint, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn globals_keep_perspective_row_flat() {
        // Regresja: czwarty element każdej kolumny tworzy wiersz `w` macierzy.
        // Jeśli w nim wyląduje niezerowa wartość (np. 1.0 z `c[2][2]`), to
        // `w` = 2 dla wierzchołków o z = 0 i dzielenie perspektywiczne
        // w NDC zmniejszy CAŁĄ scenę do połowy.
        // kamera: przesunięcie środka okna do (0,0), potem skala
        let g = Globals::new(Mat3::from_scale(Vec2::new(0.01, 0.01))
            * Mat3::from_translation(Vec2::new(-400.0, -300.0)));
        // wiersz w
        let w_row = [g.view_proj[0][3], g.view_proj[1][3], g.view_proj[2][3], g.view_proj[3][3]];
        assert_eq!(w_row, [0.0, 0.0, 0.0, 1.0], "wiersz w musi być (0,0,0,1)");
        // 200 px na prawo od środka okna -> ndc.x = 2 (poza ekranem)
        let p = g.transform_point(Vec2::new(600.0, 300.0));
        assert!((p.x - 2.0).abs() < 1e-5, "ndc.x = {:?}, oczekiwano 2.0", p.x);
        // a 100 px od środku mieści się w kadrze
        let q = g.transform_point(Vec2::new(500.0, 300.0));
        assert!((q.x - 1.0).abs() < 1e-5, "ndc.x = {:?}, oczekiwano 1.0", q.x);
    }

    #[test]
    fn globals_hold_orthographic_matrix() {
        let g = Globals::new(Mat3::from_scale(Vec2::new(2.0, 3.0)));
        assert_eq!(g.view_proj[0][0], 2.0);
        assert_eq!(g.view_proj[1][1], 3.0);
        assert_eq!(g.view_proj[3][3], 1.0);
        assert_eq!(std::mem::size_of::<Globals>(), 64);
    }

    #[test]
    fn globals_keep_camera_translation() {
        // Regresja: translacja w macierzy 2x3 siedzi w ostatniej KOLUMNIE,
        // więc po homogenizacji musi trafić do ostatniej kolumny 4x4.
        let g = Globals::new(Mat3::from_translation(Vec2::new(100.0, -50.0)));
        assert_eq!(g.transform_point(Vec2::new(10.0, 10.0)), Vec2::new(110.0, -40.0));
        assert_eq!(g.transform_point(Vec2::ZERO), Vec2::new(100.0, -50.0));
    }

    #[test]
    fn globals_scale_and_translate_together() {
        // typowa macierz kamery 2D: przesunięcie, potem skala
        let view_proj = Mat3::from_scale(Vec2::new(0.01, 0.01))
            * Mat3::from_translation(-Vec2::new(400.0, 300.0));
        let g = Globals::new(view_proj);
        // środek okna (400, 300) w pikselach -> (0, 0) w NDC
        assert!(g.transform_point(Vec2::new(400.0, 300.0)).abs().max_element() < 1e-4);
        // 100 px w prawo od środka -> 1.0 w NDC
        let right = g.transform_point(Vec2::new(500.0, 300.0));
        assert!((right.x - 1.0).abs() < 1e-4, "było {right:?}");
    }

    #[test]
    fn sort_groups_by_layer_blend_texture() {
        let a = TextureKey::Image(HandleId::new(1, 0));
        let b = TextureKey::Image(HandleId::new(2, 0));

        let mut list = DrawList::new();
        list.push_sprite(sprite(a, 0));
        list.push_sprite(sprite(b, 1));
        list.push_sprite(sprite(a, 0));
        list.sort();

        // warstwa 0 (obie z teksturą a), potem warstwa 1
        assert_eq!(list.sprites()[0].texture, a);
        assert_eq!(list.sprites()[1].texture, a);
        assert_eq!(list.sprites()[2].texture, b);
        assert_eq!(list.sprites()[2].layer, 1);
    }

    #[test]
    fn sort_is_stable_within_equal_keys() {
        let key = TextureKey::White;
        let mut list = DrawList::new();
        for x in 0..5 {
            let mut s = sprite(key, 0);
            s.transform = Mat3::from_translation(Vec2::new(x as f32, 0.0));
            list.push_sprite(s);
        }
        list.sort();
        // translacja jest w trzeciej kolumnie macierzy
        let xs: Vec<f32> = list
            .sprites()
            .iter()
            .map(|s| s.transform.to_cols_array_2d()[2][0])
            .collect();
        assert_eq!(xs, vec![0.0, 1.0, 2.0, 3.0, 4.0], "kolejność dodania musi zostać");
    }

    #[test]
    fn clear_keeps_geometry_arena_empty() {
        let mut list = DrawList::new();
        list.push_mesh_geometry(MeshGeometry::from_mesh(&Mesh::triangle(
            Vec2::ZERO,
            Vec2::X,
            Vec2::Y,
            Color::WHITE,
        )));
        assert_eq!(list.geometry().len(), 1);
        list.clear();
        assert_eq!(list.geometry().len(), 0);
        assert!(list.is_empty());
    }

    #[test]
    fn mesh_hash_detects_changes() {
        let a = Mesh::triangle(Vec2::ZERO, Vec2::X, Vec2::Y, Color::WHITE);
        let mut b = a.clone();
        assert_eq!(hash_mesh(&a), hash_mesh(&b));
        b.vertices[1].position = [5.0, 0.0];
        assert_ne!(hash_mesh(&a), hash_mesh(&b));

        let mut c = a.clone();
        c.indices = vec![0, 1, 2, 0, 2, 3];
        assert_ne!(hash_mesh(&a), hash_mesh(&c));
    }
}
