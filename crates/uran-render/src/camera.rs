//! Kamera 2D: projekcja ortograficzna, przejścia świat <-> ekran.

use uran_math::{Mat3, Rect, Vec2};

/// Kamera 2D z osią Y skierowaną w górę.
///
/// Domyślnie `zoom = 1.0` oznacza, że **1 jednostka świata = 1 piksel**,
/// czyli współrzędne świata pokrywają się z pikselami okna.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera2d {
    /// Środek widoku w przestrzeni świata.
    pub position: Vec2,
    /// Powiększenie (2.0 = dwa razy większe).
    pub zoom: f32,
    /// Obrót kamery w radianach.
    pub rotation: f32,
}

impl Default for Camera2d {
    fn default() -> Self {
        Self::new()
    }
}

impl Camera2d {
    pub const fn new() -> Self {
        Self { position: Vec2::ZERO, zoom: 1.0, rotation: 0.0 }
    }

    pub fn at(x: f32, y: f32) -> Self {
        Self { position: Vec2::new(x, y), ..Self::new() }
    }

    pub fn with_position(mut self, position: Vec2) -> Self {
        self.position = position;
        self
    }

    pub fn with_zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom.max(f32::EPSILON);
        self
    }

    pub fn with_rotation(mut self, rotation: f32) -> Self {
        self.rotation = rotation;
        self
    }

    /// Ustawia widok tak, żeby pokrywał dokładnie podany obszar świata.
    ///
    /// Przyjmuje rozmiar okna, bo od niego zależy wynikowy zoom.
    pub fn fit_world(&mut self, world: Rect, window_size: Vec2) {
        if world.is_empty() {
            return;
        }
        self.position = world.center();
        let size = world.size();
        // skalujemy tą osią, która jest ciasniejsza
        self.zoom = (window_size.x / size.x).min(window_size.y / size.y).max(f32::EPSILON);
    }

    /// Rozmiar widocznego obszaru w jednostkach świata.
    ///
    /// Przy `zoom = 1.0` równy rozmiarowi okna w pikselach.
    pub fn visible_size(&self, window_size: Vec2) -> Vec2 {
        window_size / self.zoom.max(f32::EPSILON)
    }

    /// Widoczny prostokąt w przestrzeni świata (bez obrotu — AABB).
    pub fn visible_rect(&self, window_size: Vec2) -> Rect {
        Rect::from_center(self.position, self.visible_size(window_size))
    }

    /// Macierz świata -> NDC.
    pub fn view_projection(&self, window_size: Vec2) -> Mat3 {
        // Kolejność ma znaczenie (macierze kolumnowe: w `A * B * v` najpierw
        // działa `B`):
        //   1. przesuwamy środek widoku do (0,0) -> pozycja kamery
        //   2. obracamy i skalujemy               -> zoom / rotacja
        //   3. skalujemy piksele na NDC [-1, 1]
        //
        // Uwaga na znak zoomu: `zoom = 2` oznacza 2x powiększenie, czyli
        // przesunięcie świata jest MNOŻONE — widzimy wtedy połowę sceny.
        // To zgadza się z `visible_size() = okno / zoom`.
        let view = Mat3::from_translation(-self.position)
            * Mat3::from_angle(self.rotation)
            * Mat3::from_scale(Vec2::splat(self.zoom));
        let projection = Mat3::from_scale(Vec2::new(2.0 / window_size.x, 2.0 / window_size.y));
        projection * view
    }

    /// Wygładzone podążanie za punktem (ważne dla kamery w grze).
    ///
    /// `smoothness` to tempo dochodzenia do celu; większa wartość = szybciej.
    pub fn smooth_follow(&mut self, target: Vec2, dt: f32, smoothness: f32) {
        let t = 1.0 - (-smoothness.max(0.0) * dt).exp();
        self.position = self.position.lerp(target, t);
    }

    /// Punkt ekranu (piksele, lewy górny róg = 0,0) -> punkt świata.
    pub fn screen_to_world(&self, screen: Vec2, window_size: Vec2) -> Vec2 {
        let ndc = Vec2::new(
            screen.x / window_size.x * 2.0 - 1.0,
            1.0 - screen.y / window_size.y * 2.0,
        );
        (self.view_projection(window_size).inverse() * ndc.extend(1.0)).truncate()
    }

    /// Punkt świata -> punkt ekranu (piksele, lewy górny róg = 0,0).
    pub fn world_to_screen(&self, world: Vec2, window_size: Vec2) -> Vec2 {
        let ndc = self.view_projection(window_size) * world.extend(1.0);
        Vec2::new(
            (ndc.x * 0.5 + 0.5) * window_size.x,
            (0.5 - ndc.y * 0.5) * window_size.y,
        )
    }
}

/// Ekranowa macierz projekcji: (0,0) = lewy górny róg okna, oś Y w dół.
///
/// Służy do rysowania HUD-u i UI, które nie powinno ruszać się wraz z kamerą.
///
/// Kolejność ma znaczenie: `T(-1, 1) * S(2/w, -2/h)` daje
/// `ndc = (2x/w - 1, 1 - 2y/h)`, czyli dokładnie zamianę „piksele Y w dół"
/// na NDC. Odwrócenie czynników (`S * T`) przesunęłoby HUD poza ekran,
/// bo przesunięcie zostałoby przeskalowane.
pub fn screen_matrix(window_size: Vec2) -> Mat3 {
    Mat3::from_translation(Vec2::new(-1.0, 1.0))
        * Mat3::from_scale(Vec2::new(2.0 / window_size.x, -2.0 / window_size.y))
}

#[cfg(test)]
mod screen_matrix_tests {
    use super::*;

    /// Należy używać dzielenia perspektywicznego tak samo jak w shaderze.
    fn project(m: &Mat3, p: Vec2) -> Vec2 {
        ((*m) * p.extend(1.0)).truncate()
    }

    #[test]
    fn corners_map_to_ndc_corners() {
        let size = Vec2::new(800.0, 600.0);
        let m = screen_matrix(size);
        // lewy górny róg -> (-1, 1)
        let tl = project(&m, Vec2::ZERO);
        assert!((tl.x + 1.0).abs() < 1e-5 && (tl.y - 1.0).abs() < 1e-5, "{tl:?}");
        // prawy dolny -> (1, -1)
        let br = project(&m, size);
        assert!((br.x - 1.0).abs() < 1e-5 && (br.y + 1.0).abs() < 1e-5, "{br:?}");
    }

    #[test]
    fn center_maps_to_origin_and_axis_y_points_down() {
        let size = Vec2::new(800.0, 600.0);
        let m = screen_matrix(size);
        let c = project(&m, size * 0.5);
        assert!(c.abs().max_element() < 1e-5, "środek to NDC (0,0), było {c:?}");
        // 100 px niżej -> mniejsze NDC.y (oś Y w dół)
        let below = project(&m, Vec2::new(400.0, 400.0));
        assert!(below.y < 0.0, "y=400 powinno być poniżej środka, było {below:?}");
    }

    #[test]
    fn pixels_map_one_to_one() {
        // 1 piksel przesunięcia w HUD-zie = 1 piksel na ekranie
        let size = Vec2::new(800.0, 600.0);
        let m = screen_matrix(size);
        let a = project(&m, Vec2::new(100.0, 100.0));
        let b = project(&m, Vec2::new(101.0, 100.0));
        let dx_px = (b.x - a.x) * size.x * 0.5;
        assert!((dx_px - 1.0).abs() < 1e-4, "1 px dało {dx_px} px");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Vec2 = Vec2::new(800.0, 600.0);

    #[test]
    fn center_of_window_maps_to_origin() {
        let camera = Camera2d::new();
        let world = camera.screen_to_world(Vec2::new(400.0, 300.0), WINDOW);
        assert!(world.abs().max_element() < 1e-4, "środek okna to (0,0), było {world:?}");
    }

    #[test]
    fn corners_map_to_visible_rect() {
        let camera = Camera2d::new();
        let bottom_left = camera.screen_to_world(Vec2::new(0.0, 600.0), WINDOW);
        let top_right = camera.screen_to_world(Vec2::new(800.0, 0.0), WINDOW);
        assert!((bottom_left.x + 400.0).abs() < 1e-3);
        assert!((bottom_left.y + 300.0).abs() < 1e-3);
        assert!((top_right.x - 400.0).abs() < 1e-3);
        assert!((top_right.y - 300.0).abs() < 1e-3);
        assert_eq!(camera.visible_size(WINDOW), WINDOW);
    }

    #[test]
    fn y_axis_points_up() {
        let camera = Camera2d::new();
        // suwak w dół ekranu = mniejsze Y w świecie
        let lower = camera.screen_to_world(Vec2::new(400.0, 400.0), WINDOW);
        assert!(lower.y < 0.0);
    }

    #[test]
    fn camera_position_shifts_view() {
        let camera = Camera2d::at(100.0, 50.0);
        let world = camera.screen_to_world(Vec2::new(400.0, 300.0), WINDOW);
        assert!((world.x - 100.0).abs() < 1e-3);
        assert!((world.y - 50.0).abs() < 1e-3);
        assert_eq!(camera.visible_rect(WINDOW).center(), Vec2::new(100.0, 50.0));
    }

    #[test]
    fn zoom_halves_visible_area() {
        let camera = Camera2d::new().with_zoom(2.0);
        assert_eq!(camera.visible_size(WINDOW), Vec2::new(400.0, 300.0));
        let world = camera.screen_to_world(Vec2::new(400.0, 300.0), WINDOW);
        assert!(world.abs().max_element() < 1e-4);
    }

    #[test]
    fn world_screen_roundtrip() {
        let camera = Camera2d::at(10.0, -20.0).with_zoom(1.7).with_rotation(0.3);
        let world = Vec2::new(123.0, -456.0);
        let screen = camera.world_to_screen(world, WINDOW);
        let back = camera.screen_to_world(screen, WINDOW);
        assert!((back - world).length() < 1e-2, "{back:?} != {world:?}");
    }

    #[test]
    fn zoom_changes_what_we_see() {
        let camera = Camera2d::new().with_zoom(2.0);
        // przy zoom 2 widzimy połowę sceny
        assert_eq!(camera.visible_size(WINDOW), Vec2::new(400.0, 300.0));
        // lewy kraniec okna odpowiada x = -200 (a nie -400)
        let edge = camera.screen_to_world(Vec2::new(0.0, 300.0), WINDOW);
        assert!((edge.x + 200.0).abs() < 1e-3, "edge = {edge:?}");
        assert!(edge.y.abs() < 1e-3);
    }

    #[test]
    fn fit_world_sets_zoom() {
        let mut camera = Camera2d::new();
        // okno 800x600, scena 400x300 -> zoom 2 (widzimy połowę sceny)
        camera.fit_world(Rect::from_xywh(0.0, 0.0, 400.0, 300.0), Vec2::new(800.0, 600.0));
        assert_eq!(camera.position, Vec2::new(200.0, 150.0));
        assert!((camera.zoom - 2.0).abs() < 1e-4, "zoom = {}", camera.zoom);
        // i faktycznie cała scena mieści się w widoku
        assert_eq!(camera.visible_size(Vec2::new(800.0, 600.0)), Vec2::new(400.0, 300.0));
    }

    #[test]
    fn smooth_follow_converges() {
        let mut camera = Camera2d::new();
        for _ in 0..1000 {
            camera.smooth_follow(Vec2::new(100.0, 0.0), 1.0 / 60.0, 5.0);
        }
        assert!((camera.position.x - 100.0).abs() < 0.01, "kamera nie dogoniła celu");
    }
}
