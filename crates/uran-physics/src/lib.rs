//! # Kolizje 2D — AABB
//!
//! Najprostszy system kolizji, jaki da się napisać **poprawnie**: oś‑wyrównane
//! prostokąty (AABB) i „move‑and‑slide” (przesuń po osi X, rozwiąż kolizję,
//! przesuń po osi Y, rozwiąż kolizję).
//!
//! ## Dlaczego własny, a nie `rapier2d`
//!
//! Workspace deklaruje `rapier2d`, ale go **nie używa**: do tej gry wystarczą
//! statyczne ściany i jedna kolizja na ciało na klatkę. Fizyka z napędem
//! (kontakty, tarcie, pęd) to ciężar bez pokrycia w rozgrywce, a własny kod
//! da się przetestować bez renderera — tak jak reszta silnika.
//!
//! ## Semantyka osi
//!
//! [`Rect`] używa osi Y **w górę** (`bottom` < `top`), jak cały silnik. Wektor
//! ruchu `delta` to przyrost pozycji w jednostkach świata na klatkę.
//!
//! ## Co tu jest, a czego nie ma
//!
//! - [`Solid`]: statyczna przeszkoda (ściana, skała) — prostokąt.
//! - [`CollisionWorld`]: zbiór przeszkód + [`CollisionWorld::move_body`],
//!   które przesuwa ciało i **nie pozwala** mu wejść w żadną przeszkodę,
//!   ślizgając się po jej krawędzi.
//! - Nie ma ciał dynamicznych (tylko statyczne przeszkody i ruchomy gość w
//!   [`CollisionWorld::move_body`]). Dwa ruchome ciała to inna para kolizji.
//!
//! ## Testowalne bez renderera
//!
//! `CollisionWorld` operuje na `Rect` i `Vec2` — testy sprawdzają geometrię:
//! ściana zatrzymuje, narożnik nie „wciąga”, a ruch po przekątnej ślizga się.

use uran_math::{Rect, Vec2};

mod rapier_world;

pub use rapier_world::{BodyKind, PhysicsBody, PhysicsWorld};

/// Statyczna przeszkoda — oś‑wyrównany prostokąt.
///
/// Oddzielny typ zamiast gołego [`Rect`] po to, by przyszłe pola (np. „czy
/// przez to można przejść, ale nie przelecieć”) nie wymagały zmiany sygnatur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Solid {
    /// Obszar przeszkody w jednostkach świata.
    pub rect: Rect,
}

impl Solid {
    /// Nowa przeszkoda o danym prostokącie.
    pub const fn new(rect: Rect) -> Self {
        Self { rect }
    }

    /// Przeszkoda od punktu `origin` (lewy dolny róg) o wymiarach `size`.
    pub fn from_xywh(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            rect: Rect::from_xywh(x, y, w, h),
        }
    }
}

/// Zbiór statycznych przeszkód — świat, po którym porusza się ciało.
#[derive(Debug, Clone, Default)]
pub struct CollisionWorld {
    solids: Vec<Solid>,
}

impl CollisionWorld {
    /// Pusty świat.
    pub fn new() -> Self {
        Self { solids: Vec::new() }
    }

    /// Dodaje przeszkodę.
    pub fn add(&mut self, solid: Solid) {
        self.solids.push(solid);
    }

    /// Wszystkie przeszkody (do rysowania i debugowania).
    pub fn solids(&self) -> &[Solid] {
        &self.solids
    }

    /// Czy prostokąt `body` nachodzi na **jakąkolwiek** przeszkodę.
    pub fn overlaps(&self, body: Rect) -> bool {
        self.solids.iter().any(|s| s.rect.intersects(&body))
    }

    /// Przesuwa ciało o `delta`, **rozwiązując kolizje** z przeszkodami.
    ///
    /// Ruch dzielimy na **podkroki** nie większe niż połowa mniejszego wymiaru
    /// ciała. To jest tanie „swept AABB”: duży krok nie przeskoczy przez cienką
    /// ścianę, bo każdy podkrok jest mniejszy niż ciało.
    ///
    /// W każdym podkroku: przesuń o `d.x`, cofnij z przeszkód po X, potem
    /// przesuń o `d.y`, cofnij po Y. Rozdzielenie osi daje **ślizganie się**
    /// po ścianie — składowa wzdłuż ściany przechodzi, składowa w ścianę jest
    /// blokowana, a ciało nie przykleja się do narożnika.
    pub fn move_body(&self, body: Rect, delta: Vec2) -> Rect {
        // Bez przeszkód ruch jest dokładny — substepping wprowadzałby tylko
        // błąd zaokrąglenia (`3.0` robiło się `2.9999998`).
        if self.solids.is_empty() {
            return body.translate(delta);
        }
        let steps = self.step_count(body, delta);
        let d = delta / steps as f32;
        let mut moved = body;
        for _ in 0..steps {
            moved = self.step_body(moved, d);
        }
        moved
    }

    /// Ile podkroków trzeba, by żaden nie był większy niż połowa ciała.
    fn step_count(&self, body: Rect, delta: Vec2) -> usize {
        let step = (body.width().min(body.height()) * 0.5).max(0.1);
        (delta.length() / step).ceil().max(1.0) as usize
    }

    /// Jeden podkrok ruchu: X, kolizja, Y, kolizja.
    fn step_body(&self, body: Rect, d: Vec2) -> Rect {
        let mut moved = body.translate(Vec2::new(d.x, 0.0));
        moved = self.resolve_axis(moved, Axis::X, d.x);
        moved = moved.translate(Vec2::new(0.0, d.y));
        moved = self.resolve_axis(moved, Axis::Y, d.y);
        moved
    }

    /// Cofa ciało z przeszkód wzdłuż jednej osi, **znając kierunek ruchu**.
    ///
    /// Kierunek (`dir`) decyduje, w którą stronę wypychamy: ciało idące w
    /// prawo, które weszło w ścianę, cofamy w lewo do krawędzi ściany.
    /// Dzięki temu ciało w środku przeszkody nie „przeskakuje" na drugą
    /// stronę, tylko wraca tam, skąd przyszło.
    ///
    /// Pętla z licznikiem, nie pojedynczy przebieg: jedna przeszkoda może
    /// wepchnąć ciało w drugą, a wtedy kolejna iteracja musi je cofnąć dalej.
    fn resolve_axis(&self, body: Rect, axis: Axis, dir: f32) -> Rect {
        let mut resolved = body;
        for _ in 0..self.solids.len() + 1 {
            let Some(hit) = self.solids.iter().find(|s| s.rect.intersects(&resolved)) else {
                break;
            };
            resolved = push_out(resolved, hit.rect, axis, dir);
        }
        resolved
    }
}

/// Oś, wzdłuż której rozwiązujemy kolizję.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// Wypycha `body` z `solid` wzdłuż jednej osi do najbliższej krawędzi.
///
/// `dir` to kierunek ruchu na tej osi przed kolizją: `dir >= 0` znaczy „szło
/// w prawo / w górę", więc cofamy w lewo / w dół.
fn push_out(body: Rect, solid: Rect, axis: Axis, dir: f32) -> Rect {
    match axis {
        Axis::X => {
            let dx = if dir >= 0.0 {
                solid.min.x - body.max.x
            } else {
                solid.max.x - body.min.x
            };
            body.translate(Vec2::new(dx, 0.0))
        }
        Axis::Y => {
            let dy = if dir >= 0.0 {
                solid.min.y - body.max.y
            } else {
                solid.max.y - body.min.y
            };
            body.translate(Vec2::new(0.0, dy))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(x: f32, y: f32, w: f32, h: f32) -> Solid {
        Solid::from_xywh(x, y, w, h)
    }

    /// Ciało 1x1 w (0,0), pchnięte w prawo o 10, zatrzymuje się na ścianie
    /// stojącej na x = 5.
    #[test]
    fn a_wall_stops_horizontal_motion() {
        let mut world = CollisionWorld::new();
        world.add(wall(5.0, -1.0, 1.0, 3.0));

        let body = Rect::from_xywh(0.0, 0.0, 1.0, 1.0);
        let moved = world.move_body(body, Vec2::new(10.0, 0.0));

        assert!(
            (moved.max.x - 5.0).abs() < 1e-3,
            "cialo przeszlo przez sciane: {moved:?}"
        );
    }

    /// Ściana zatrzymuje też ruch w dół.
    #[test]
    fn a_wall_stops_vertical_motion() {
        let mut world = CollisionWorld::new();
        world.add(wall(-10.0, 0.0, 20.0, 1.0));

        let body = Rect::from_xywh(0.0, 5.0, 1.0, 1.0);
        let moved = world.move_body(body, Vec2::new(0.0, -10.0));

        assert!(
            (moved.min.y - 1.0).abs() < 1e-3,
            "cialo wpadlo pod podloge: {moved:?}"
        );
    }

    /// Ruch po przekątnej wzdłuż ściany **ślizga się**: składowa Y przechodzi,
    /// a X jest zablokowana — ciało nie „przykleja się" do narożnika.
    #[test]
    fn diagonal_motion_slides_along_the_wall() {
        let mut world = CollisionWorld::new();
        world.add(wall(5.0, -10.0, 1.0, 20.0));

        let body = Rect::from_xywh(0.0, 0.0, 1.0, 1.0);
        let moved = world.move_body(body, Vec2::new(10.0, 10.0));

        assert!((moved.max.x - 5.0).abs() < 1e-3);
        // Ciało 1x1 startuje na y = 0..1 (środek 0.5), więc po ruchu o 10
        // środek ląduje na 10.5 — nie na 10.0.
        assert!(
            (moved.center().y - 10.5).abs() < 1e-3,
            "Y nie przeszlo: {moved:?}"
        );
    }

    /// Ciało już **w** przeszkodzie zostaje z niej wypchnięte po mniejszej osi.
    #[test]
    fn a_body_inside_a_solid_is_pushed_out() {
        let mut world = CollisionWorld::new();
        world.add(wall(0.0, 0.0, 10.0, 10.0));

        let body = Rect::from_xywh(1.0, 1.0, 1.0, 1.0);
        let moved = world.move_body(body, Vec2::new(0.0, 0.0));

        assert!(
            !moved.intersects(&Rect::from_xywh(0.0, 0.0, 10.0, 10.0)),
            "cialo zostalo w przeszkodzie: {moved:?}"
        );
    }

    /// Duży krok nie „tuneluje" przez przeszkodę.
    #[test]
    fn a_large_step_does_not_tunnel_through() {
        let mut world = CollisionWorld::new();
        world.add(wall(0.0, 0.0, 100.0, 100.0));

        let body = Rect::from_xywh(-50.0, 50.0, 1.0, 1.0);
        let moved = world.move_body(body, Vec2::new(200.0, 0.0));

        assert!(
            moved.max.x.abs() < 1e-3,
            "cialo przeszlo przez przeszkode: {moved:?}"
        );
    }

    /// Pusty świat nie rusza ciała.
    #[test]
    fn an_empty_world_does_not_move_the_body() {
        let world = CollisionWorld::new();
        let body = Rect::from_xywh(0.0, 0.0, 1.0, 1.0);
        let moved = world.move_body(body, Vec2::new(3.0, 4.0));
        assert_eq!(moved, Rect::from_xywh(3.0, 4.0, 1.0, 1.0));
    }

    /// `overlaps` zgłasza kolizję z każdą przeszkodą.
    #[test]
    fn overlaps_detects_solids() {
        let mut world = CollisionWorld::new();
        world.add(wall(5.0, 5.0, 1.0, 1.0));
        assert!(world.overlaps(Rect::from_xywh(5.5, 5.5, 1.0, 1.0)));
        assert!(!world.overlaps(Rect::from_xywh(0.0, 0.0, 1.0, 1.0)));
    }
}
