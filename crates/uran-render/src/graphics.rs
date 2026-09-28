//! Tryb natychmiastowy: `Graphics` to „pędzel" do rysowania w klatce.
//!
//! Współrzędne podawane w metodach są **lokalne** — obowiązuje stos
//! transformacji, więc cały HUD albo wirujący się element da się objąć
//! jednym `push_transform` / `pop_transform`.

use uran_asset::{FontData, Handle, Image};
use uran_ecs::{BlendMode, Mesh, UvRect};
use uran_math::{Color, Mat3, Rect, Vec2};

use crate::batch::{
    DrawList, MeshDraw, MeshGeometry, NineSliceDraw, SpriteDraw, TextAlign, TextDraw, TextureKey,
};
use crate::shape;

/// Bieżące ustawienia rysowania (kolor, tryb mieszania, warstwa).
#[derive(Debug, Clone, Copy)]
pub struct DrawStyle {
    pub color: Color,
    pub blend: BlendMode,
    pub layer: i32,
}

impl Default for DrawStyle {
    fn default() -> Self {
        Self { color: Color::WHITE, blend: BlendMode::Alpha, layer: 0 }
    }
}

/// Pędzel rysujący do `DrawList`.
pub struct Graphics<'a> {
    list: &'a mut DrawList,
    /// Stos macierzy: `stack[0]` to transformacja świata.
    stack: Vec<Mat3>,
    style: DrawStyle,
    /// Czy kolejne rysowania idą w przestrzeni ekranu (HUD/UI).
    screen_space: bool,
    draw_calls: usize,
}

impl<'a> Graphics<'a> {
    pub fn new(list: &'a mut DrawList) -> Self {
        Self {
            list,
            stack: vec![Mat3::IDENTITY],
            style: DrawStyle::default(),
            screen_space: false,
            draw_calls: 0,
        }
    }

    /// Bieżąca macierz transformacji (w przestrzeni świata).
    pub fn transform(&self) -> Mat3 {
        *self.stack.last().expect("stos transformacji nie może być pusty")
    }

    // --- Style ---
    //
    // Metody stylu zwracają `&mut Self`, więc da się ich używać zarówno
    // w łańcuchu (`gfx.color(c).blend(b).draw_rect(r)`), jak i osobno
    // (`gfx.layer(10);`).

    /// Kolor bieżący (tint) dla kolejnych rysowań.
    pub fn color(&mut self, color: Color) -> &mut Self {
        self.style.color = color;
        self
    }

    /// Tryb mieszania dla kolejnych rysowań.
    pub fn blend(&mut self, blend: BlendMode) -> &mut Self {
        self.style.blend = blend;
        self
    }

    /// Warstwa sortowania — wyższa rysuje się później (na wierzchu).
    pub fn layer(&mut self, layer: i32) -> &mut Self {
        self.style.layer = layer;
        self
    }

    pub fn style(&self) -> DrawStyle {
        self.style
    }

    // --- Przestrzeń rysowania ---

    /// Przechodzi w przestrzeń ekranu: (0,0) to lewy górny róg okna,
    /// oś Y w dół, a pozycja nie zależy od kamery. Służy do HUD-u i UI.
    ///
    /// Czyści stos transformacji — UI rysujemy zawsze „od zera", inaczej
    /// przejęłoby przesunięcia zostawione przez scenę (np. po cząsteczkach).
    pub fn screen_space(&mut self) -> &mut Self {
        self.screen_space = true;
        self.stack.clear();
        self.stack.push(Mat3::IDENTITY);
        self
    }

    /// Powrót do przestrzeni świata (domyślne), także z czystym stosem.
    pub fn world_space(&mut self) -> &mut Self {
        self.screen_space = false;
        self.stack.clear();
        self.stack.push(Mat3::IDENTITY);
        self
    }

    pub fn is_screen_space(&self) -> bool {
        self.screen_space
    }

    // --- Transformacje ---

    /// Dokłada transformację na wierzch stosu.
    pub fn push_transform(&mut self, transform: Mat3) -> &mut Self {
        let current = self.transform();
        self.stack.push(current * transform);
        self
    }

    pub fn pop_transform(&mut self) -> &mut Self {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
        self
    }

    pub fn translate(&mut self, offset: Vec2) -> &mut Self {
        self.push_transform(Mat3::from_translation(offset))
    }

    pub fn rotate(&mut self, radians: f32) -> &mut Self {
        self.push_transform(Mat3::from_angle(radians))
    }

    pub fn scale(&mut self, factor: Vec2) -> &mut Self {
        self.push_transform(Mat3::from_scale(factor))
    }

    /// Obraca układ współrzędnych wokół wskazanego punktu.
    ///
    /// Macierz to `T(p) * R * T(-p)` — dla macierzy kolumnowych
    /// `A * B * v` najpierw działa `B`, więc punkt jest najpierw przesuwany
    /// do początku, potem obracany, a na końcu przenoszony z powrotem.
    pub fn rotate_about(&mut self, origin: Vec2, radians: f32) -> &mut Self {
        self.push_transform(
            Mat3::from_translation(origin)
                * Mat3::from_angle(radians)
                * Mat3::from_translation(-origin),
        )
    }

    /// Wraca do transformacji tożsamościowej.
    pub fn reset_transform(&mut self) -> &mut Self {
        self.stack.truncate(1);
        self
    }

    pub fn transform_depth(&self) -> usize {
        self.stack.len()
    }

    // --- Rysowanie ---

    /// Prostokąt wypełniony kolorem (`rect` w lokalnych współrzędnych).
    pub fn draw_rect(&mut self, rect: Rect) -> &mut Self {
        let mut sprite =
            SpriteDraw::rect(rect, self.transform(), UvRect::FULL, self.style.color);
        sprite.blend = self.style.blend;
        sprite.layer = self.style.layer;
        sprite.screen_space = self.screen_space;
        self.list.push_sprite(sprite);
        self.draw_calls += 1;
        self
    }

    /// Kontur prostokąta o zadanej grubości.
    pub fn draw_rect_outline(&mut self, rect: Rect, width: f32) -> &mut Self {
        self.draw_mesh(&shape::rect_outline(rect, width, self.style.color))
    }

    /// Prostokąt z zaokrąglonymi rogami.
    pub fn draw_rounded_rect(&mut self, rect: Rect, radius: f32) -> &mut Self {
        self.draw_mesh(&shape::rounded_rect(rect, radius, 4, self.style.color))
    }

    /// Okrąg wypełniony.
    pub fn draw_circle(&mut self, center: Vec2, radius: f32) -> &mut Self {
        self.draw_mesh(&shape::circle(center, radius, 32, self.style.color))
    }

    /// Pierścień.
    pub fn draw_ring(&mut self, center: Vec2, radius: f32, thickness: f32) -> &mut Self {
        self.draw_mesh(&shape::ring(center, radius, thickness, 32, self.style.color))
    }

    /// Odcinek o grubości.
    pub fn draw_line(&mut self, from: Vec2, to: Vec2, width: f32) -> &mut Self {
        self.draw_mesh(&shape::thick_line(from, to, width, self.style.color))
    }

    /// Łamana o grubości.
    pub fn draw_polyline(&mut self, points: &[Vec2], width: f32) -> &mut Self {
        self.draw_mesh(&shape::polyline(points, width, self.style.color))
    }

    /// Wypełniony wielokąt (triangulowany automatycznie).
    pub fn draw_polygon(&mut self, points: &[Vec2]) -> &mut Self {
        self.draw_mesh(&shape::polygon(points, self.style.color))
    }

    /// Trójkąt.
    pub fn draw_triangle(&mut self, a: Vec2, b: Vec2, c: Vec2) -> &mut Self {
        self.draw_mesh(&Mesh::triangle(a, b, c, self.style.color))
    }

    /// Dowolna siatka (bierze bieżący kolor, transformację i warstwę).
    pub fn draw_mesh(&mut self, mesh: &Mesh) -> &mut Self {
        if mesh.is_empty() || !mesh.validate() {
            return self;
        }
        let geometry = self.list.push_mesh_geometry(MeshGeometry::from_mesh(mesh));
        let draw = MeshDraw {
            geometry,
            transform: self.transform(),
            color: self.style.color,
            texture: TextureKey::White,
            blend: self.style.blend,
            layer: self.style.layer,
            screen_space: self.screen_space,
        };
        self.list.meshes_mut().push(draw);
        self.draw_calls += 1;
        self
    }

    // --- Tekstury ---

    /// Tekstura w zadanym prostokącie świata.
    pub fn draw_texture(&mut self, texture: Handle<Image>, rect: Rect, uv: UvRect) -> &mut Self {
        let mut sprite = SpriteDraw::rect(rect, self.transform(), uv, self.style.color);
        sprite.texture = TextureKey::Image(texture.into());
        sprite.blend = self.style.blend;
        sprite.layer = self.style.layer;
        sprite.screen_space = self.screen_space;
        self.list.push_sprite(sprite);
        self.draw_calls += 1;
        self
    }

    /// Sprite wyśrodkowany w punkcie (anchor = środek).
    pub fn draw_sprite(
        &mut self,
        texture: Handle<Image>,
        center: Vec2,
        size: Vec2,
    ) -> &mut Self {
        self.draw_texture(texture, Rect::from_center(center, size), UvRect::FULL)
    }

    /// Cała tekstura w prostokącie.
    pub fn draw_texture_full(&mut self, texture: Handle<Image>, rect: Rect) -> &mut Self {
        self.draw_texture(texture, rect, UvRect::FULL)
    }

    /// Tekstura 9-slice: rogi się nie rozciągają, środek jest wypełniany.
    ///
    /// Idealne do ramek UI. `border` to margines w **pikselach obrazu
    /// źródłowego** (np. 8 dla ramki 16x16). Rozwijaniem na 9 prostokątów
    /// zajmuje się renderer, bo tylko on zna rozmiar tekstury.
    pub fn draw_texture_9slice(
        &mut self,
        texture: Handle<Image>,
        dst: Rect,
        border: f32,
    ) -> &mut Self {
        self.list.push_nine_slice(NineSliceDraw {
            texture,
            dst,
            transform: self.transform(),
            border,
            color: self.style.color,
            blend: self.style.blend,
            layer: self.style.layer,
            screen_space: self.screen_space,
        });
        self.draw_calls += 1;
        self
    }

    // --- Tekst ---

    /// Tekst — rysowany dopiero w trakcie renderowania (potrzebuje atlasu).
    ///
    /// `position` to początek pierwszej linii bazowej przy [`TextAlign::Left`];
    /// przy wyrównaniu do środka lub w prawo jest odpowiednio środkiem
    /// albo prawym końcem linii.
    pub fn draw_text(
        &mut self,
        font: Handle<FontData>,
        text: &str,
        position: Vec2,
        size: f32,
        align: TextAlign,
    ) -> &mut Self {
        self.push_text(font, text, position, size, align, None)
    }

    /// Tekst z zawijaniem wierszy przy `max_width`.
    pub fn draw_text_wrapped(
        &mut self,
        font: Handle<FontData>,
        text: &str,
        position: Vec2,
        size: f32,
        align: TextAlign,
        max_width: f32,
    ) -> &mut Self {
        self.push_text(font, text, position, size, align, Some(max_width))
    }

    fn push_text(
        &mut self,
        font: Handle<FontData>,
        text: &str,
        position: Vec2,
        size: f32,
        align: TextAlign,
        max_width: Option<f32>,
    ) -> &mut Self {
        // pozycja idzie przez bieżącą transformację
        let world = (self.transform() * position.extend(1.0)).truncate();
        self.list.push_text(TextDraw {
            font,
            text: text.into(),
            position: world,
            size,
            color: self.style.color,
            layer: self.style.layer,
            blend: self.style.blend,
            align,
            line_height: 1.0,
            max_width,
            screen_space: self.screen_space,
        });
        self.draw_calls += 1;
        self
    }

    /// Liczba wywołań rysowania w tej klatce.
    pub fn draw_calls(&self) -> usize {
        self.draw_calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gfx(list: &mut DrawList) -> Graphics<'_> {
        Graphics::new(list)
    }

    #[test]
    fn draw_rect_goes_through_transform_stack() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.translate(Vec2::new(100.0, 0.0));
            g.color(Color::RED);
            g.draw_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0));
        }
        let sprites = list.sprites();
        assert_eq!(sprites.len(), 1);
        // prostokąt 0..10 przesunięty o 100 -> środek 105
        assert!(((sprites[0].transform * Vec2::ZERO.extend(1.0)).truncate() - Vec2::new(105.0, 5.0)).length() < 1e-4);
        assert_eq!(sprites[0].color, Color::RED);
    }

    #[test]
    fn world_and_screen_sprites_are_never_mixed_in_one_run() {
        // HUD i scena mają różne macierze, więc `screen_space` musi rozdzielać
        // przebiegi rysowania. Gdyby nie rozdzielał, cały HUD dostałby macierz
        // świata i pojechałby w złym miejscu.
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.world_space().color(Color::WHITE).draw_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0));
            g.screen_space().color(Color::WHITE).draw_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0));
        }
        list.sort();

        // obie są białymi sprite'ami w tej samej warstwie, więc jedynym
        // różnicującym je kluczem jest `screen_space`
        assert_eq!(list.sprites().len(), 2);
        assert!(!list.sprites()[0].screen_space, "świat musi być najpierw");
        assert!(list.sprites()[1].screen_space, "HUD zawsze na wierzchu");
        assert_eq!(list.sprites()[0].layer, list.sprites()[1].layer);
        assert_eq!(list.sprites()[0].blend, list.sprites()[1].blend);
        assert_eq!(list.sprites()[0].texture, list.sprites()[1].texture);
    }

    #[test]
    fn screen_space_is_remembered_by_graphics() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            assert!(!g.is_screen_space(), "domyślnie rysujemy w świecie");
            g.screen_space();
            assert!(g.is_screen_space());
            g.color(Color::WHITE).draw_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0));
            g.world_space();
            assert!(!g.is_screen_space());
            g.color(Color::WHITE).draw_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0));
        }
        assert!(list.sprites()[0].screen_space);
        assert!(!list.sprites()[1].screen_space);
    }

    #[test]
    fn switching_space_resets_transform_stack() {
        // UI nie może dziedziczyć przesunięć zostawionych przez scenę.
        let mut list = DrawList::new();
        let transform = {
            let mut g = gfx(&mut list);
            g.translate(Vec2::new(500.0, 300.0)); // "zanieczyszczenie" ze sceny
            let _ = g.transform();
            g.screen_space();
            g.color(Color::WHITE).draw_rect(Rect::from_xywh(10.0, 20.0, 30.0, 40.0));
            g.world_space();
            g.color(Color::WHITE).draw_rect(Rect::from_xywh(10.0, 20.0, 30.0, 40.0));
            g.transform()
        };
        assert_eq!(transform, Mat3::IDENTITY, "stos transformacji musi być czysty");
    }

    #[test]
    fn rotate_about_origin() {
        let mut list = DrawList::new();
        let transform = {
            let mut g = gfx(&mut list);
            g.rotate_about(Vec2::new(50.0, 50.0), std::f32::consts::FRAC_PI_2);
            let t = g.transform();
            // żeby `sprites()` nie było puste dla czytelności testu
            g.draw_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0));
            t
        };
        // punkt 1 px na prawo od środka ma wylądować 1 px NAD nim
        let p = (transform * Vec2::new(51.0, 50.0).extend(1.0)).truncate();
        assert!((p - Vec2::new(50.0, 51.0)).length() < 1e-3, "było {p:?}");
    }

    #[test]
    fn pop_transform_restores() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.translate(Vec2::new(10.0, 0.0));
            assert_eq!(g.transform_depth(), 2);
            g.pop_transform();
            assert_eq!(g.transform_depth(), 1);
            // nadmiarowe pop nie może wyczyścić stosu
            g.pop_transform();
            g.pop_transform();
            assert_eq!(g.transform_depth(), 1);
            assert_eq!(g.transform(), Mat3::IDENTITY);
        }
    }

    #[test]
    fn shapes_add_meshes() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.draw_circle(Vec2::ZERO, 10.0);
            g.draw_line(Vec2::ZERO, Vec2::new(10.0, 0.0), 2.0);
            g.draw_polygon(&[Vec2::ZERO, Vec2::X, Vec2::Y]);
        }
        assert_eq!(list.meshes().len(), 3);
        assert!(list.geometry().iter().all(|g| !g.indices.is_empty()));
    }

    #[test]
    fn nine_slice_is_recorded_for_renderer() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.draw_texture_9slice(Handle::new(0, 0), Rect::from_xywh(0.0, 0.0, 100.0, 40.0), 8.0);
        }
        assert_eq!(list.nine_slices().len(), 1);
        assert_eq!(list.nine_slices()[0].border, 8.0);
    }

    #[test]
    fn text_is_recorded_with_style() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.layer(5).color(Color::YELLOW);
            g.draw_text(Handle::new(0, 0), "witaj", Vec2::new(10.0, 20.0), 16.0, TextAlign::Center);
        }
        let texts = list.texts();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].text.as_ref(), "witaj");
        assert_eq!(texts[0].layer, 5);
        assert_eq!(texts[0].align, TextAlign::Center);
        assert_eq!(texts[0].position, Vec2::new(10.0, 20.0));
    }

    #[test]
    fn layer_and_blend_are_applied() {
        let mut list = DrawList::new();
        {
            let mut g = gfx(&mut list);
            g.layer(3).blend(BlendMode::Additive);
            g.draw_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0));
        }
        assert_eq!(list.sprites()[0].layer, 3);
        assert_eq!(list.sprites()[0].blend, BlendMode::Additive);
    }
}
