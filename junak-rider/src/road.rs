//! Prosta, nieskończona droga: nawierzchnia, oznakowanie, pobocze.
//!
//! Droga jest jednym pasmem zbudowanym RAZ w przestrzeni lokalnej.
//! „Nieskończoność" zapewnia przesuwanie motocyzlu w dół osi Z i
//! cofnięcie go o stały krok, gdy zje daleko (patrz `RECENTER`).
//! Dzięki temu w scenie jest zawsze jedna droga zamiast tysięcy kafelków.

use uran_math::Vec3;
use uran_render3d::mesh::{Mesh, Vertex};

/// Długość pasa drogi (m). Musi być dłuższa niż dystans cofnięcia,
/// inaczej przed motocyklem zrobiłaby się dziura.
pub const ROAD_LEN: f32 = 600.0;

/// Po przekroczeniu tej pozycji cofamy świat o `RECENTER_STEP`.
///
/// Numer dobierany tak, żeby po cofnięciu pozycja znowu mieściła się
/// w pasie z zapasem na widoczność do przodu i do tyłu.
pub const RECENTER_AT: f32 = 200.0;
pub const RECENTER_STEP: f32 = 400.0;

/// Szerokość jezdni (m) — dwa pasy po 3,6 m plus pobocza.
pub const ROAD_HALF: f32 = 3.6;
/// Odległość krawędzi oznakowania od osi.
pub const EDGE_LINE: f32 = 3.3;
/// Szerokość pasa odbojowego przy krawędzi.
const SHOULDER_W: f32 = 1.2;
/// Szerokość trawnika/pobocza poza jezdnią.
const VERGE_W: f32 = 40.0;

/// Trzy siatki drogi — osobne, bo osobne kolory.
pub struct RoadMeshes {
    pub asphalt: Mesh,
    pub verge: Mesh,
    pub paint: Mesh,
}

/// Kolory podane w sRGB, tak jak reszta silnika.
const ASPHALT: [f32; 3] = [0.20, 0.205, 0.215];
/// Drobna jaśniejsza paska co jadę — asfalt nigdy nie jest jednolity.
const ASPHALT_WORN: [f32; 3] = [0.235, 0.238, 0.245];
const VERGE: [f32; 3] = [0.30, 0.34, 0.20];
const PAINT: [f32; 3] = [0.86, 0.87, 0.84];

/// Buduje trzy siatki drogi w przestrzeni lokalnej.
///
/// Wszystkie wierzchołki dostają normalną `[0,1,0]` i UV, bo `Vertex`
/// wymaga ich obu, a `uv` przyda się, gdy docelowym materiał będzie
/// teksturą (asfalt z szumu zamiast płaskiego koloru).
pub fn build_road() -> RoadMeshes {
    let mut asphalt = Mesh::new();
    let mut verge = Mesh::new();
    let mut paint = Mesh::new();

    let z0 = -ROAD_LEN * 0.5;
    let z1 = ROAD_LEN * 0.5;

    // --- jezdnia: dwie szerokie pasy, lekko różne odcienie
    for (side, col) in [(1.0f32, ASPHALT), (-1.0f32, ASPHALT_WORN)] {
        let x = side * ROAD_HALF;
        quad_xz(
            &mut asphalt,
            Vec3::new(0.0, 0.0, z0),
            Vec3::new(x, 0.0, z0),
            Vec3::new(x, 0.0, z1),
            Vec3::new(0.0, 0.0, z1),
            col,
        );
    }

    // --- pobocza po obu stronach, 1,2 m pasm przy krawędzi
    for side in [1.0f32, -1.0] {
        let x0 = side * ROAD_HALF;
        let x1 = side * (ROAD_HALF + SHOULDER_W);
        quad_xz(
            &mut asphalt,
            Vec3::new(x0, 0.0, z0),
            Vec3::new(x1, 0.0, z0),
            Vec3::new(x1, 0.0, z1),
            Vec3::new(x0, 0.0, z1),
            ASPHALT_WORN,
        );
    }

    // --- trawnik: szeroki pas po każdej stronie
    for side in [1.0f32, -1.0] {
        let x0 = side * (ROAD_HALF + SHOULDER_W);
        let x1 = side * (ROAD_HALF + SHOULDER_W + VERGE_W);
        quad_xz(
            &mut verge,
            Vec3::new(x0, 0.0, z0),
            Vec3::new(x1, 0.0, z0),
            Vec3::new(x1, 0.0, z1),
            Vec3::new(x0, 0.0, z1),
            VERGE,
        );
    }

    // --- linie brzegowe (ciągłe)
    for side in [1.0f32, -1.0] {
        stripe(
            &mut paint,
            side * (EDGE_LINE - 0.075),
            side * (EDGE_LINE + 0.075),
            z0,
            z1,
        );
    }

    // --- linia osi: przerywana, 3 m pasa co 9 m
    //
    // Przerwa jest dłuższa niż pasek (6 m), bo tak wygląda oznakowanie
    // na drogach szybkiego ruchu — i od razu widać prędkość.
    let mut z = z0;
    while z < z1 {
        let end = (z + 3.0).min(z1);
        stripe(&mut paint, -0.075, 0.075, z, end);
        z += 9.0;
    }

    RoadMeshes {
        asphalt,
        verge,
        paint,
    }
}

/// Prostokątna łata na płaszczyźnie Y=0, podana czterema rogami w kolejności
/// zgodnej z ruchem wskazówek (liczymy je normalnym w górę).
///
/// ## Dlaczego NIE ufamy kolejności rogów
///
/// Łaty drogi są lustrzanymi odbiciami siebie nawzajem: prawa pół
/// jezdni to lewa odwrócona w X. Przy lustrzanym odbiciu zmienia się
/// znak wyznacznika, czyli WINDĄ w trójkącie zmienia się zgodność —
/// i połowa drogi znika przy `cull_mode: Back`.
///
/// Dlatego liczymy iloczyn wektorowy i sami porządkujemy wierzchołki.
/// Dzięki temu caller podaje „cztery rogi łaty" w dowolnej kolejności
/// i zawsze dostaje trójkąty zwrócone do góry.
fn quad_xz(mesh: &mut Mesh, a: Vec3, b: Vec3, c: Vec3, d: Vec3, col: [f32; 3]) {
    // UV rozciągamy wzdłuż osi Z, żeby ewentualna tekstura asfaltu
    // nie rozciągała się na 600 m
    let uv = |p: Vec3| [p.x * 0.5, p.z * 0.08];
    let n = Vec3::Y;

    // Iloczyn wektorowy wskazuje w górę tylko dla jednej z dwóch
    // kolejności wierzchołków.
    let up = (b - a).cross(c - a).y > 0.0;
    let (b, d) = if up { (b, d) } else { (d, b) };

    let v0 = Vertex::new_uv(a, n, uv(a), col);
    let v1 = Vertex::new_uv(b, n, uv(b), col);
    let v2 = Vertex::new_uv(c, n, uv(c), col);
    let v3 = Vertex::new_uv(d, n, uv(d), col);
    let i = mesh.vertices.len() as u32;
    mesh.vertices.extend_from_slice(&[v0, v1, v2, v3]);
    mesh.indices
        .extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
}

/// Pionowy pasek oznakowania między `x0` a `x1`, od `z0` do `z1`.
fn stripe(mesh: &mut Mesh, x0: f32, x1: f32, z0: f32, z1: f32) {
    quad_xz(
        mesh,
        Vec3::new(x0, 0.0, z0),
        Vec3::new(x1, 0.0, z0),
        Vec3::new(x1, 0.0, z1),
        Vec3::new(x0, 0.0, z1),
        PAINT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sprawdza, że KAŻDY trójkąt siatki ma windę zgodną z `+Y`.
    ///
    /// To najdroższy możliwy błąd w tej grze: połowa drogi znika,
    /// a na ekranie wygląda to jak „droga się kończy" albo „kamera
    /// jest krzywo" — a przyczyna jest w jednej literze kolejności
    /// wierzchołków. Dlatego testujemy KAŻDY trójkąt, nie losowy.
    fn assert_all_faces_up(m: &Mesh, what: &str) {
        assert!(
            m.indices.len() % 3 == 0,
            "{what}: indeksy nie są trójkątami"
        );
        for t in m.indices.chunks_exact(3) {
            let p = |i: u32| m.vertices[i as usize].pos();
            let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            assert!(
                n.y > 0.0,
                "{what}: trójkąt {:?} ma windę w dół (n.y = {}) — zniknie przy cullingu",
                [t[0], t[1], t[2]],
                n.y
            );
        }
    }

    #[test]
    fn every_road_face_points_up() {
        let r = build_road();
        assert_all_faces_up(&r.asphalt, "asfalt");
        assert_all_faces_up(&r.verge, "pobocze");
        assert_all_faces_up(&r.paint, "oznakowanie");
    }

    #[test]
    fn road_actually_covers_both_sides() {
        // Druga strona tego samego błędu: nawet gdy winda jest dobra,
        // jedna z pół może wpaść poza `ROAD_HALF`.
        let r = build_road();
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for v in &r.asphalt.vertices {
            lo = lo.min(v.position[0]);
            hi = hi.max(v.position[0]);
        }
        assert!(
            lo <= -ROAD_HALF + 1e-3,
            "asfalt nie dochodzi do lewej krawędzi: {lo}"
        );
        assert!(
            hi >= ROAD_HALF - 1e-3,
            "asfalt nie dochodzi do prawej krawędzi: {hi}"
        );
    }

    #[test]
    fn quad_xz_accepts_corners_in_any_order() {
        // Dwie łaty lustrzane: druga ma odwróconą kolejność rogów.
        // Wynik musi być identyczny — inaczej połowa drogi znika.
        let mut m1 = Mesh::new();
        quad_xz(
            &mut m1,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 4.0),
            Vec3::new(0.0, 0.0, 4.0),
            [1.0; 3],
        );
        let mut m2 = Mesh::new();
        quad_xz(
            &mut m2,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 4.0),
            Vec3::new(2.0, 0.0, 4.0),
            Vec3::new(2.0, 0.0, 0.0),
            [1.0; 3],
        );
        assert_eq!(m1.vertices.len(), m2.vertices.len());
        assert_eq!(m1.indices, m2.indices);
        assert_all_faces_up(&m1, "łata 1");
        assert_all_faces_up(&m2, "łata lustrzana");
    }

    #[test]
    fn road_is_long_enough_for_the_recentering_step() {
        // Gdyby droga była krótsza niż krok cofnięcia, po cofnięciu
        // pojazdu znalazłby się poza nią i wypadł w pustkę.
        assert!(
            ROAD_LEN > 2.0 * RECENTER_AT,
            "droga {} m jest krótsza niż zakres cofania ±{} m",
            ROAD_LEN,
            RECENTER_AT
        );
        assert!(
            RECENTER_STEP < ROAD_LEN,
            "krok cofnięcia {RECENTER_STEP} m przeskakuje przez całą drogę"
        );
    }

    #[test]
    fn centre_line_is_dashed_not_solid() {
        // Ciągła linia osi wygląda jak ściana i zabija poczucie prędkości.
        let r = build_road();
        // oznakowanie = 2 linie brzegowe + N przerywanych; policzmy
        // trójkąty: jedna linia ciągła to 2 trójkąty na 600 m
        let tris = r.paint.indices.len() / 3;
        assert!(tris > 100, "za mało oznakowania: {tris} trójkątów");
        // przerwa 6 m na 9 m → ok. 2/3 pasa pustego
        let dashes = tris / 2 - 2;
        assert!(
            dashes > 50 && dashes < 70,
            "dashes = {dashes}, oczekiwaliśmy ~66"
        );
    }
}
