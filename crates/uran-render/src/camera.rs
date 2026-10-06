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
        Self {
            position: Vec2::ZERO,
            zoom: 1.0,
            rotation: 0.0,
        }
    }

    pub fn at(x: f32, y: f32) -> Self {
        Self {
            position: Vec2::new(x, y),
            ..Self::new()
        }
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
        self.zoom = (window_size.x / size.x)
            .min(window_size.y / size.y)
            .max(f32::EPSILON);
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
        // Kolejność ma znaczenie i jest **odwrócona** względem opisu
        // kroków, bo macierze kolumnowe w `A * B * v` najpierw działają `B`:
        //   1. przesuwamy środek widoku do (0,0) -> pozycja kamery
        //   2. obracamy i skalujemy               -> zoom / rotacja
        //   3. skalujemy piksele na NDC [-1, 1]
        //
        // Czyli matematycznie chcemy `zoom * (p - camera)`, a to wymaga
        // kolejności `S * R * T`, nie `T * R * S`.
        //
        // Podmiana tych dwóch jest bardzo podstępna: przy `T * R * S`
        // przesunięcie zostaje w **nieprzeskalowanych** jednostkach świata,
        // więc dla `zoom = 1` wszystko wygląda poprawnie, a przy większym
        // powiększeniu cała scena zjeżdża o `camera * (zoom - 1)` i kamera
        // przestaje pokazywać to, co wskazuje. Dlatego pilnujemy tego tu
        // (i w teście poniżej) zamiast ufać, że „na oko wychodzi".
        //
        // Uwaga na znak zoomu: `zoom = 2` oznacza 2x powiększenie, czyli
        // przesunięcie świata jest MNOŻONE — widzimy wtedy połowę sceny.
        // To zgadza się z `visible_size() = okno / zoom`.
        let view = Mat3::from_scale(Vec2::splat(self.zoom))
            * Mat3::from_angle(self.rotation)
            * Mat3::from_translation(-self.position);
        let projection = Mat3::from_scale(Vec2::new(2.0 / window_size.x, 2.0 / window_size.y));
        projection * view
    }

    /// Przyciąga kamerę do prostokąta świata, żeby nie pokazywać pustki
    /// poza mapą.
    ///
    /// Gdy świat jest **większy** od widoku, kamera zatrzymuje się na
    /// krawędziach — i to jest normalny przypadek w grze. Gdy świat jest
    /// **mniejszy** niż widok (mała mapa albo mocne powiększenie), nie da się
    /// dopasować krawędzi, więc środkujemy mapę i zostawiamy margines dookoła.
    ///
    /// Dzięki temu po ostatnim kaflu nie widać „czarnego" tła, a mała mapa
    /// nie ucieka do rogu ekranu.
    pub fn clamp_to_bounds(&mut self, world: Rect, window_size: Vec2) {
        if world.is_empty() {
            return;
        }
        let visible = self.visible_size(window_size);
        // Oś, na której świat mieści się w widoku, dostaje środek zamiast
        // przycięcia — inaczej `clamp` zadziałałby w drugą stronę.
        let axis = |center: f32, half_visible: f32, min: f32, max: f32| {
            if half_visible * 2.0 >= max - min {
                (min + max) * 0.5
            } else {
                center.clamp(min + half_visible, max - half_visible)
            }
        };
        self.position.x = axis(self.position.x, visible.x * 0.5, world.min.x, world.max.x);
        self.position.y = axis(self.position.y, visible.y * 0.5, world.min.y, world.max.y);
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
        assert!(
            (tl.x + 1.0).abs() < 1e-5 && (tl.y - 1.0).abs() < 1e-5,
            "{tl:?}"
        );
        // prawy dolny -> (1, -1)
        let br = project(&m, size);
        assert!(
            (br.x - 1.0).abs() < 1e-5 && (br.y + 1.0).abs() < 1e-5,
            "{br:?}"
        );
    }

    #[test]
    fn center_maps_to_origin_and_axis_y_points_down() {
        let size = Vec2::new(800.0, 600.0);
        let m = screen_matrix(size);
        let c = project(&m, size * 0.5);
        assert!(
            c.abs().max_element() < 1e-5,
            "środek to NDC (0,0), było {c:?}"
        );
        // 100 px niżej -> mniejsze NDC.y (oś Y w dół)
        let below = project(&m, Vec2::new(400.0, 400.0));
        assert!(
            below.y < 0.0,
            "y=400 powinno być poniżej środka, było {below:?}"
        );
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
        assert!(
            world.abs().max_element() < 1e-4,
            "środek okna to (0,0), było {world:?}"
        );
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
        camera.fit_world(
            Rect::from_xywh(0.0, 0.0, 400.0, 300.0),
            Vec2::new(800.0, 600.0),
        );
        assert_eq!(camera.position, Vec2::new(200.0, 150.0));
        assert!((camera.zoom - 2.0).abs() < 1e-4, "zoom = {}", camera.zoom);
        // i faktycznie cała scena mieści się w widoku
        assert_eq!(
            camera.visible_size(Vec2::new(800.0, 600.0)),
            Vec2::new(400.0, 300.0)
        );
    }

    #[test]
    fn smooth_follow_converges() {
        let mut camera = Camera2d::new();
        for _ in 0..1000 {
            camera.smooth_follow(Vec2::new(100.0, 0.0), 1.0 / 60.0, 5.0);
        }
        assert!(
            (camera.position.x - 100.0).abs() < 0.01,
            "kamera nie dogoniła celu"
        );
    }

    /// Świat mniejszy od okna (jak farma 640x480 przy oknie 1280x720):
    /// krawędzi nie da się dopasować, więc mapa ma być na środku.
    #[test]
    fn clamp_centers_world_smaller_than_view() {
        let mut camera = Camera2d::at(-500.0, 900.0);
        let world = Rect::from_xywh(0.0, 0.0, 640.0, 480.0);
        camera.clamp_to_bounds(world, Vec2::new(1280.0, 720.0));
        assert_eq!(camera.position, world.center());
    }

    /// Świat większy od okna: kamera zatrzymuje się na krawędzi, więc po
    /// ostatnim kaflu nie widać pustki.
    #[test]
    fn clamp_stops_at_big_world_edges() {
        let mut camera = Camera2d::at(5000.0, -4000.0);
        let world = Rect::from_xywh(0.0, 0.0, 4000.0, 3000.0);
        let window = Vec2::new(800.0, 600.0);
        camera.clamp_to_bounds(world, window);
        // Widok 800x600 przy zoom 1 -> środek nie może wyjść poza
        // 400 od prawej i 300 od dolnej krawędzi.
        assert!(
            (camera.position.x - 3600.0).abs() < 1e-3,
            "{}",
            camera.position.x
        );
        assert!(
            (camera.position.y - 300.0).abs() < 1e-3,
            "{}",
            camera.position.y
        );

        let view = camera.visible_rect(window);
        assert!(view.min.x >= world.min.x - 1e-3 && view.max.x <= world.max.x + 1e-3);
        assert!(view.min.y >= world.min.y - 1e-3 && view.max.y <= world.max.y + 1e-3);
    }

    /// Środek mapy przy dużym świecie ma zostać tam, gdzie był.
    #[test]
    fn clamp_keeps_center_inside_big_world() {
        let mut camera = Camera2d::at(1234.0, 2345.0);
        let world = Rect::from_xywh(0.0, 0.0, 4000.0, 3000.0);
        camera.clamp_to_bounds(world, Vec2::new(800.0, 600.0));
        assert!((camera.position.x - 1234.0).abs() < 1e-3);
        assert!((camera.position.y - 2345.0).abs() < 1e-3);
    }

    /// Pusty świat nie może zepsuć kamery (brak prostokąta = brak clampu).
    #[test]
    fn clamp_ignores_empty_world() {
        let mut camera = Camera2d::at(10.0, 20.0);
        camera.clamp_to_bounds(Rect::ZERO, Vec2::new(800.0, 600.0));
        assert_eq!(camera.position, Vec2::new(10.0, 20.0));
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;

    /// Środek widoku zawsze ląduje w środku okna — niezależnie od zoomu.
    ///
    /// To jest test na kolejność mnożenia w `view_projection`. Przy
    /// błędnej kolejności (`T * R * S`) przesunięcie nie jest mnożone przez
    /// zoom, więc środek świata ucieka z ekranu przy `zoom != 1`, a przy
    /// `zoom = 1` wszystko wygląda dobrze i błąd zostaje niezauważony.
    #[test]
    fn camera_center_is_screen_center_at_any_zoom() {
        let window = Vec2::new(800.0, 600.0);
        for zoom in [0.25f32, 0.5, 1.0, 1.75, 3.0, 10.0] {
            let cam = Camera2d::at(1000.0, 700.0).with_zoom(zoom);
            let screen = cam.world_to_screen(cam.position, window);
            assert!(
                (screen.x - window.x * 0.5).abs() < 1e-3,
                "zoom={zoom}: x środka to {screen:?}"
            );
            assert!(
                (screen.y - window.y * 0.5).abs() < 1e-3,
                "zoom={zoom}: y środka to {screen:?}"
            );
        }
    }

    /// Świat przesunięty o jednostkę musi przesunąć się o `zoom` pikseli.
    ///
    /// Test na to sam błąd, ale bezpośrednio: różnica dwóch punktów nie
    /// zawiera pozycji kamery, więc da się sprawdzić sam czynnik skalujący.
    #[test]
    fn world_offset_is_scaled_by_zoom() {
        let window = Vec2::new(800.0, 600.0);
        for zoom in [0.5f32, 1.0, 2.0, 4.0] {
            let cam = Camera2d::at(1000.0, 700.0).with_zoom(zoom);
            let a = cam.world_to_screen(Vec2::new(1000.0, 700.0), window);
            let b = cam.world_to_screen(Vec2::new(1100.0, 700.0), window);
            assert!(
                (b.x - a.x - 100.0 * zoom).abs() < 1e-3,
                "zoom={zoom}: przesunięcie wynosi {} zamiast {}",
                b.x - a.x,
                100.0 * zoom
            );
        }
    }

    /// `fit_world` + `world_to_screen` daje cały świat na ekranie.
    ///
    /// To sprawdza cały łańcuch naraz: gdy `zoom` jest liczony dla innego
    /// rozmiaru okna niż ten, w którym renderujemy, świat nie mieści się
    /// w kadrze i zjeżdża do rogu.
    #[test]
    fn fit_world_fills_the_window() {
        let window = Vec2::new(800.0, 600.0);
        let world = Rect::from_xywh(0.0, 0.0, 1920.0, 1080.0);
        let mut cam = Camera2d::new();
        cam.fit_world(world, window);

        let top_left = cam.world_to_screen(world.min, window);
        let bottom_right = cam.world_to_screen(world.max, window);

        // Świat szeroki niż okno wypełnia je poziomo i jest przycięty w pionie.
        assert!((top_left.x).abs() < 1e-2, "lewy krawędź na {top_left:?}");
        assert!(
            (bottom_right.x - window.x).abs() < 1e-2,
            "prawy krawędź na {bottom_right:?}"
        );
        // Środek świata jest dokładnie w środku okna.
        let center = cam.world_to_screen(world.center(), window);
        assert!((center.x - window.x * 0.5).abs() < 1e-2);
        assert!((center.y - window.y * 0.5).abs() < 1e-2);
    }

    /// Świat mniejszy od okna jest wyśrodkowany, a nie przycięty.
    #[test]
    fn small_world_is_centered_not_clipped() {
        let window = Vec2::new(800.0, 600.0);
        let world = Rect::from_xywh(0.0, 0.0, 100.0, 100.0);
        let mut cam = Camera2d::new();
        cam.fit_world(world, window);
        cam.clamp_to_bounds(world, window);

        let center = cam.world_to_screen(world.center(), window);
        assert!((center.x - window.x * 0.5).abs() < 1e-2);
        assert!((center.y - window.y * 0.5).abs() < 1e-2);
    }

    /// Ruch w prawo po świecie daje ruch w prawo na ekranie.
    ///
    /// Test „na oko" chroniący przed odwróceniem osi (np. `Y w dół`), co
    /// w 2D top-down jest bardzo łatwo przegapić.
    #[test]
    fn screen_and_world_axes_agree() {
        let window = Vec2::new(800.0, 600.0);
        let cam = Camera2d::at(500.0, 500.0).with_zoom(1.5);
        let base = cam.world_to_screen(cam.position, window);

        let right = cam.world_to_screen(cam.position + Vec2::new(10.0, 0.0), window);
        assert!(right.x > base.x, "świat w prawo = ekran w prawo");

        // oś Y świata idzie w górę, a na ekranie w górę
        let up = cam.world_to_screen(cam.position + Vec2::new(0.0, 10.0), window);
        assert!(up.y < base.y, "świat w górę = ekran w górę");
    }

    /// `screen_to_world` i `world_to_screen` są nawzajem odwrotne.
    #[test]
    fn world_screen_roundtrip() {
        let window = Vec2::new(1024.0, 768.0);
        let cam = Camera2d::at(640.0, 480.0).with_zoom(1.3);
        for point in [
            Vec2::new(0.0, 0.0),
            Vec2::new(320.0, 240.0),
            Vec2::new(1024.0, 768.0),
            Vec2::new(1280.0, 96.0),
        ] {
            let back = cam.screen_to_world(cam.world_to_screen(point, window), window);
            assert!((back - point).length() < 1e-2, "{point:?} -> {back:?}");
        }
    }
}
