//! Bryły 3D budowane proceduralnie — brak plików modeli.
//!
//! Każda bryła to lista ścian (cztery narożniki + kolor), z których
//! `box_mesh` generuje wierzchołki z poprawnymi normalnymi. Normalne
//! są **dzielone przez narożniki**, żeby sąsiednie ściany nie miały
//! gładszych krawędzi — inaczej prostopadłościan wygląda jak bańka mydlana.

use uran_math::{Vec3, Vec4};
use uran_render3d::{Mesh, Vertex};

/// Jedna ściana: cztery narożniki w kolejności zgodnej z ruchem
/// wskazówek zegara widzianym od zewnątrz + kolor.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub corners: [Vec3; 4],
    pub color: [f32; 3],
}

impl Face {
    /// Ściana z trzech punktów (czwarty domyka).
    pub fn tri(a: Vec3, b: Vec3, c: Vec3, color: [f32; 3]) -> Face {
        Face { corners: [a, b, c, a], color }
    }

    /// Ściana definiowana wektorami przesunięcia z punktu bazowego.
    /// Kolejność `u` x `v` musi dawać normalną skierowaną na zewnątrz.
    pub fn quad(origin: Vec3, u: Vec3, v: Vec3, color: [f32; 3]) -> Face {
        Face { corners: [origin, origin + u, origin + u + v, origin + v], color }
    }
}

/// Normalna ściany z pierwszych trzech narożników (Newell).
fn face_normal(f: &Face) -> Vec3 {
    let n = (f.corners[1] - f.corners[0]).cross(f.corners[2] - f.corners[0]);
    n.normalize_or_zero()
}

/// Zbudowuje siatkę z listy ścian.
///
/// Ściany o znormalizowanych współrzędnych mają wspólne wierzchołki
/// (pozycje się pokrywają), ale każda ma własne trzy — dzięki temu
/// normalna jest płaska na całej ścianie.
pub fn faces_to_mesh(faces: &[Face]) -> Mesh {
    let mut mesh = Mesh::new();
    for f in faces {
        let n = face_normal(f);
        // dwie pozycje z jednej trójki: [0,1,2] i [0,2,3]
        let v0 = Vertex::new(f.corners[0], n, f.color);
        let v1 = Vertex::new(f.corners[1], n, f.color);
        let v2 = Vertex::new(f.corners[2], n, f.color);
        mesh.tri(v0, v1, v2);
        let v0b = Vertex::new(f.corners[0], n, f.color);
        let v2b = Vertex::new(f.corners[2], n, f.color);
        let v3 = Vertex::new(f.corners[3], n, f.color);
        mesh.tri(v0b, v2b, v3);
    }
    mesh
}

/// Prostopadłościan wypełniony ścianami.
///
/// `half` to połowa wymiarów, `center` — środek. Ściany są w kolejności
/// CCW widzianej z zewnątrz, bo potok renderuje tylko `Face::Back`.
pub fn box_faces(center: Vec3, half: Vec3, color: [f32; 3]) -> Vec<Face> {
    let p = [
        center + Vec3::new(-half.x, -half.y, -half.z),
        center + Vec3::new(half.x, -half.y, -half.z),
        center + Vec3::new(half.x, half.y, -half.z),
        center + Vec3::new(-half.x, half.y, -half.z),
        center + Vec3::new(-half.x, -half.y, half.z),
        center + Vec3::new(half.x, -half.y, half.z),
        center + Vec3::new(half.x, half.y, half.z),
        center + Vec3::new(-half.x, half.y, half.z),
    ];
    vec![
        // tył (-Z) i przód (+Z)
        Face { corners: [p[0], p[3], p[2], p[1]], color },
        Face { corners: [p[4], p[5], p[6], p[7]], color },
        // lewo (-X) i prawo (+X)
        Face { corners: [p[0], p[1], p[5], p[4]], color },
        Face { corners: [p[3], p[7], p[6], p[2]], color },
        // dół (-Y) i góra (+Y)
        Face { corners: [p[0], p[4], p[7], p[3]], color },
        Face { corners: [p[1], p[2], p[6], p[5]], color },
    ]
}

/// Prostopadłościan gotowy do wgrania.
pub fn box_mesh(center: Vec3, half: Vec3, color: [f32; 3]) -> Mesh {
    faces_to_mesh(&box_faces(center, half, color))
}

// --------------------------------------------------------------- czołg

/// Wymiary czołgu w jednostkach świata.
///
/// Kadłub szeroki i niski (jak w World of Tanks), gąsiennice wystają
/// na boki. Wszystko w metrach-ish, żeby zasięgi i kamera miały sens.
pub mod tank_dim {
    /// Półdługość kadłuba (oś Z; przód dodatni).
    pub const HULL_HALF_Z: f32 = 3.2;
    /// Półszerokość kadłuba.
    pub const HULL_HALF_X: f32 = 1.5;
    /// Półwysokość kadłuba.
    pub const HULL_HALF_Y: f32 = 0.55;
    /// Wysokość podwozia nad ziemią.
    pub const TRACK_Y: f32 = 0.45;
    /// Półszerokość gąsiennicy.
    pub const TRACK_HALF_X: f32 = 0.42;
    /// Półwysokość gąsiennicy.
    pub const TRACK_HALF_Y: f32 = 0.45;
    /// Środek wieży w osi Y.
    pub const TURRET_Y: f32 = 1.05;
    /// Półwymiary wieży (x, y, z).
    pub const TURRET_HALF: [f32; 3] = [1.15, 0.42, 1.35];
    /// Długość lufy od osi wieży.
    pub const GUN_LEN: f32 = 2.6;
    /// Półgrubość lufy.
    pub const GUN_HALF: f32 = 0.14;
    /// Środek lufy w osi Z (od osi wieży).
    pub const GUN_Z: f32 = GUN_LEN * 0.5;
}

/// Kolory drużyn (zwykły RGB, shader miesza w liniowym).
pub mod palette {
    pub const PLAYER: [f32; 3] = [0.36, 0.52, 0.30];
    pub const ENEMY: [f32; 3] = [0.55, 0.32, 0.24];
    pub const TRACK: [f32; 3] = [0.17, 0.17, 0.19];
    pub const GUN: [f32; 3] = [0.26, 0.27, 0.28];
    pub const GROUND: [f32; 3] = [0.34, 0.38, 0.24];
    pub const ROCK: [f32; 3] = [0.42, 0.40, 0.38];
}

/// Kadłub czołgu (bez wieży i gąsiennic).
///
/// Przednia ściana jest pochylona do tyłu — stąd `front_top`. Bez tego
/// bryła wygląda jak klocek, a pochylenie daje czytelny „pysk".
pub fn tank_hull(color: [f32; 3]) -> Mesh {
    use tank_dim as d;
    let w = d::HULL_HALF_X;
    let y_bot = d::TRACK_Y;
    let y_top = d::TRACK_Y + d::HULL_HALF_Y * 2.0;
    let z = d::HULL_HALF_Z;

    // Ściany w kolejności CCW widzianej Z ZEWNĄTRZ. Kolejność wierzchołków
    // jest tu krytyczna: `Face` liczy normalną jako
    // `(c1 - c0) × (c2 - c0)`, więc odwrócenie wierzchołków 2 i 3
    // odwraca ścianę i cała bryła znika przy back-face culling.
    let faces = vec![
        Face { corners: [ // tył (-Z)
            Vec3::new(-w, y_bot, -z), Vec3::new(-w, y_top, -z),
            Vec3::new(w, y_top, -z), Vec3::new(w, y_bot, -z),
        ], color },
        Face { corners: [ // lewo (-X)
            Vec3::new(-w, y_bot, -z), Vec3::new(-w, y_bot, z),
            Vec3::new(-w, y_top, z), Vec3::new(-w, y_top, -z),
        ], color },
        Face { corners: [ // prawo (+X)
            Vec3::new(w, y_bot, -z), Vec3::new(w, y_top, -z),
            Vec3::new(w, y_top, z), Vec3::new(w, y_bot, z),
        ], color },
        Face { corners: [ // dół (-Y)
            Vec3::new(-w, y_bot, -z), Vec3::new(w, y_bot, -z),
            Vec3::new(w, y_bot, z), Vec3::new(-w, y_bot, z),
        ], color },
        Face { corners: [ // góra (+Y)
            Vec3::new(-w, y_top, -z), Vec3::new(-w, y_top, z),
            Vec3::new(w, y_top, z), Vec3::new(w, y_top, -z),
        ], color },
        Face { corners: [ // przód (+Z), pochylony do tyłu u góry
            Vec3::new(-w, y_bot, z), Vec3::new(w, y_bot, z),
            Vec3::new(w * 0.88, y_top, z - 0.5), Vec3::new(-w * 0.88, y_top, z - 0.5),
        ], color },
    ];
    faces_to_mesh(&faces)
}

/// Gąsiennica po jednej stronie kadłuba (`side` = -1 lub +1).
pub fn tank_track(side: f32) -> Mesh {
    use tank_dim as d;
    let x = side * (d::HULL_HALF_X + d::TRACK_HALF_X * 0.5);
    box_mesh(
        Vec3::new(x, d::TRACK_Y, 0.0),
        Vec3::new(d::TRACK_HALF_X, d::TRACK_HALF_Y, d::HULL_HALF_Z * 0.95),
        palette::TRACK,
    )
}

/// Koło napędowe (krótki walec na końcu gąsiennicy).
pub fn tank_wheel(side: f32, z: f32) -> Mesh {
    use tank_dim as d;
    let x = side * (d::HULL_HALF_X + d::TRACK_HALF_X * 1.15);
    box_mesh(
        Vec3::new(x, d::TRACK_Y, z),
        Vec3::new(d::TRACK_HALF_X * 0.45, d::TRACK_HALF_Y * 0.6, 0.22),
        palette::TRACK,
    )
}

/// Wieża. Środek w jej osi — gry obraca ją wokół Y.
///
/// Grzbiet jest lekko pochylony do tyłu, żeby wieża nie była idealnym
/// prostopadłościanem; od razu widać, że to coś ciekawego.
///
/// Kolejność wierzchołków na każdej ścianie musi dawać normalną na
/// zewnątrz (`(c1-c0) × (c2-c0)`), inaczej ściana znika przy culling —
/// to sprawdzają testy `normals_point_away_from_centre`.
pub fn tank_turret(color: [f32; 3]) -> Mesh {
    use tank_dim as d;
    let [hx, hy, hz] = d::TURRET_HALF;
    let faces = vec![
        Face { corners: [Vec3::new(-hx, -hy, -hz), Vec3::new(-hx, hy, -hz), // tył (-Z)
                         Vec3::new(hx, hy, -hz), Vec3::new(hx, -hy, -hz)], color },
        Face { corners: [Vec3::new(-hx, -hy, -hz), Vec3::new(-hx, -hy, hz), // lewo (-X)
                         Vec3::new(-hx, hy, hz), Vec3::new(-hx, hy, -hz)], color },
        Face { corners: [Vec3::new(hx, -hy, -hz), Vec3::new(hx, hy, -hz), // prawo (+X)
                         Vec3::new(hx, hy, hz), Vec3::new(hx, -hy, hz)], color },
        Face { corners: [Vec3::new(-hx, -hy, -hz), Vec3::new(hx, -hy, -hz), // dół (-Y)
                         Vec3::new(hx, -hy, hz), Vec3::new(-hx, -hy, hz)], color },
        Face { corners: [Vec3::new(-hx, -hy, hz), Vec3::new(hx, -hy, hz), // przód, pochylony
                         Vec3::new(hx * 0.85, hy, hz - 0.3), Vec3::new(-hx * 0.85, hy, hz - 0.3)], color },
        Face { corners: [Vec3::new(-hx, hy, -hz), Vec3::new(-hx, hy, hz - 0.3), // grzbiet
                         Vec3::new(hx, hy, hz - 0.3), Vec3::new(hx, hy, -hz)], color },
    ];
    faces_to_mesh(&faces)
}

/// Lufa — wychodzi z osi wieży w kierunku +Z (przód czołgu).
pub fn tank_gun() -> Mesh {
    use tank_dim as d;
    box_mesh(
        Vec3::new(0.0, 0.0, d::GUN_Z),
        Vec3::new(d::GUN_HALF, d::GUN_HALF, d::GUN_LEN * 0.5),
        palette::GUN,
    )
}

/// Wieżyczka dowódcy na dachu wieży.
pub fn tank_cupola(color: [f32; 3]) -> Mesh {
    use tank_dim as d;
    box_mesh(
        Vec3::new(-0.35, d::TURRET_HALF[1] + 0.22, -0.25),
        Vec3::new(0.30, 0.22, 0.30),
        color,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_has_six_faces_and_twelve_triangles() {
        let f = box_faces(Vec3::ZERO, Vec3::splat(1.0), [1.0; 3]);
        assert_eq!(f.len(), 6, "prostopadłościan ma 6 ścian");
        let mesh = faces_to_mesh(&f);
        // 6 ścian x 2 trójkąty
        assert_eq!(mesh.indices.len(), 36);
        assert_eq!(mesh.vertices.len(), 36);
    }

    #[test]
    fn box_normals_point_outward() {
        // normalna każdej ściany musi wskazywać OD środka, inaczej
        // potok z `cull_mode: Back` wyrzuci wszystkie ściany
        let faces = box_faces(Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0), [1.0; 3]);
        for f in &faces {
            let n = face_normal(f);
            let centroid = f.corners.iter().fold(Vec3::ZERO, |a, b| a + *b) / 4.0;
            assert!(
                n.dot(centroid) > 0.0,
                "normalna {n:?} nie wskazuje na zewnątrz (środek {centroid:?})"
            );
        }
    }

    #[test]
    fn gun_points_forward() {
        // lufa musi wystawać do przodu (+Z), inaczej czołg strzela w bok
        let gun = tank_gun();
        let max_z = gun.vertices.iter().map(|v| v.pos().z).fold(f32::MIN, f32::max);
        assert!(max_z > 1.0, "lufa nie wystaje: max z = {max_z}");
        assert!(tank_dim::GUN_Z > 0.0);
    }

    #[test]
    fn tracks_stick_out_from_hull() {
        // gąsiennice muszą być szersze niż kadłub
        let track = tank_track(1.0);
        let max_x = track.vertices.iter().map(|v| v.pos().x).fold(f32::MIN, f32::max);
        assert!(
            max_x > tank_dim::HULL_HALF_X,
            "gąsiennica nie wystaje poza kadłub: {max_x} vs {}",
            tank_dim::HULL_HALF_X
        );
    }

    #[test]
    fn left_and_right_tracks_mirror_each_other() {
        // Lustro = odbicie w osi X. Lewa gąsiennica ma X ujemne, więc po
        // odwróceniu sortowania i zmianie znaku musi pokryć się z prawą.
        let mut lx: Vec<f32> = tank_track(-1.0).vertices.iter().map(|v| v.pos().x).collect();
        let mut rx: Vec<f32> = tank_track(1.0).vertices.iter().map(|v| v.pos().x).collect();
        lx.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rx.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(lx.len(), rx.len());
        let n = lx.len();
        for i in 0..n {
            // najmniejsze X lewej = największe X prawej
            let mirrored = -lx[i];
            assert!(
                (mirrored - rx[n - 1 - i]).abs() < 1e-5,
                "gąsiennice nie są lustrzane: {mirrored} vs {}",
                rx[n - 1 - i]
            );
        }
    }

    #[test]
    fn normals_point_away_from_centre() {
        // Właściwy niezmiennik dla back-face culling: normalna każdego
        // wierzchołka musi wskazywać OD środka bryły.
        //
        // UWAGA: nie dawać tu "suma normalnych = 0" — to prawda tylko dla
        // brył o równych, równoległych przeciwległych ścianach. Nasz kadłub
        // ma pochyloną ścianę czołową, więc suma jednostkowych normalnych
        // jest odejściem od zera i to jest w porządku.
        for (name, mesh) in [
            ("kadłub", tank_hull(palette::PLAYER)),
            ("wieża", tank_turret(palette::PLAYER)),
            ("prostopadłościan", box_mesh(Vec3::ZERO, Vec3::splat(1.0), [1.0; 3])),
        ] {
            let centre = mesh.center();
            for (i, v) in mesh.vertices.iter().enumerate() {
                let outward = v.pos() - centre;
                assert!(
                    v.normal().dot(outward) > 0.0,
                    "{name}: wierzchołek {i} ma normalną {} wskazującą DO środka \
                     (pozycja {:?}, środek {:?})",
                    v.normal(),
                    v.pos(),
                    centre
                );
            }
        }
    }

    #[test]
    fn closed_shapes_have_no_degenerate_normals() {
        // każdy trójkąt musi mieć jednostkową, niezerową normalną —
        // zerowa normalna daje czarny piksel w miejscu, które powinno
        // być bryłą
        for (name, mesh) in [
            ("kadłub", tank_hull(palette::PLAYER)),
            ("wieża", tank_turret(palette::PLAYER)),
        ] {
            for (i, v) in mesh.vertices.iter().enumerate() {
                let len = v.normal().length();
                assert!(
                    (len - 1.0).abs() < 1e-3,
                    "{name}: wierzchołek {i} ma niedoznormalizowaną normalną {:?}",
                    v.normal()
                );
            }
        }
    }
}