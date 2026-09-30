//! Proste bryły 3D budowane w kodzie — bez plików `.obj`.
//!
//! Cała scena powstaje z `Mesh` składanych z trójkątów. Dzięki temu
//! demo nie zależy od żadnego assetu i uruchamia się z pustego
//! katalogu, a kształt każdego obiektu da się zmienić w kodzie.
//!
//! ## Wiatr (winding)
//!
//! Wszystkie ściany budujemy w kolejności dającej **normalną na
//! zewnątrz**, bo potok ma `cull_mode: Back`. Trójkąt z podwórną
//! orientacją znika razem z całą bryłą — objaw jest mylący, bo reszta
//! sceny renderuje się poprawnie.

use uran_math::Vec3;
use uran_render3d::{Mesh, Vertex};

/// Prostopadłościan: 6 ścian, każda w 2 trójkąty.
///
/// `half` to połowa rozmiaru, więc bryła jest symetryczna wokół
/// `center` — wygodniej niż podawanie dwóch rogów.
pub fn box_mesh(center: Vec3, half: Vec3, color: [f32; 3]) -> Mesh {
    let mut m = Mesh::new();
    let h = half;
    let c = center;

    // Każda ściana: 4 narożniki w kolejności dającej CCW widziany
    // z zewnątrz.
    let faces: [([Vec3; 4], Vec3); 6] = [
        // górna (+Y)
        (
            [
                c + Vec3::new(-h.x, h.y, -h.z),
                c + Vec3::new(-h.x, h.y, h.z),
                c + Vec3::new(h.x, h.y, h.z),
                c + Vec3::new(h.x, h.y, -h.z),
            ],
            Vec3::Y,
        ),
        // dolna (-Y)
        (
            [
                c + Vec3::new(-h.x, -h.y, h.z),
                c + Vec3::new(-h.x, -h.y, -h.z),
                c + Vec3::new(h.x, -h.y, -h.z),
                c + Vec3::new(h.x, -h.y, h.z),
            ],
            Vec3::NEG_Y,
        ),
        // przednia (+Z)
        (
            [
                c + Vec3::new(-h.x, -h.y, h.z),
                c + Vec3::new(h.x, -h.y, h.z),
                c + Vec3::new(h.x, h.y, h.z),
                c + Vec3::new(-h.x, h.y, h.z),
            ],
            Vec3::Z,
        ),
        // tylna (-Z)
        (
            [
                c + Vec3::new(h.x, -h.y, -h.z),
                c + Vec3::new(-h.x, -h.y, -h.z),
                c + Vec3::new(-h.x, h.y, -h.z),
                c + Vec3::new(h.x, h.y, -h.z),
            ],
            Vec3::NEG_Z,
        ),
        // prawa (+X)
        (
            [
                c + Vec3::new(h.x, -h.y, h.z),
                c + Vec3::new(h.x, -h.y, -h.z),
                c + Vec3::new(h.x, h.y, -h.z),
                c + Vec3::new(h.x, h.y, h.z),
            ],
            Vec3::X,
        ),
        // lewa (-X)
        (
            [
                c + Vec3::new(-h.x, -h.y, -h.z),
                c + Vec3::new(-h.x, -h.y, h.z),
                c + Vec3::new(-h.x, h.y, h.z),
                c + Vec3::new(-h.x, h.y, -h.z),
            ],
            Vec3::NEG_X,
        ),
    ];

    for (corners, normal) in faces {
        // UV: dolna krawędź V=0, górna V=1. Przy braku map nie ma to
        // znaczenia, ale układ jest spójny na wypadek dodania tekstury.
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for (i, corner) in corners.iter().enumerate() {
            m.push(Vertex::new_uv(*corner, normal, uvs[i], color));
        }
        m.triangle(0, 1, 2);
        m.triangle(0, 2, 3);
    }
    m
}

/// Kula (UV-sphere) o podziale `rings` × `slices`.
///
/// Używamy jej dla głowy postaci i dla emitujących elementów — kuliste
/// bryły łapią światło w sposób nieosiągalny prostopadłościanem.
pub fn sphere(center: Vec3, radius: f32, rings: u32, slices: u32, color: [f32; 3]) -> Mesh {
    let mut m = Mesh::new();
    let (ri, si) = (rings.max(2), slices.max(3));

    for r in 0..=ri {
        // `v` od 0 (biegun północny) do 1 (południowy).
        let v = r as f32 / ri as f32;
        let phi = v * std::f32::consts::PI;
        let (sin_phi, cos_phi) = phi.sin_cos();
        for s in 0..=si {
            let u = s as f32 / si as f32;
            let theta = u * std::f32::consts::TAU;
            let (sin_t, cos_t) = theta.sin_cos();
            let dir = Vec3::new(sin_phi * cos_t, cos_phi, sin_phi * sin_t);
            let p = center + dir * radius;
            // Normalna = kierunek promienia. Dla kuli to dokładnie ta
            // sama definicja co pozycja znormalizowana.
            m.push(Vertex::new_uv(p, dir, [u, 1.0 - v], color));
        }
    }

    // `si + 1` wierzchołków na pierścień — ostatni powtarza pierwszy,
    // żeby UV nie miało szwu.
    let stride = (si + 1) as u32;
    for r in 0..ri {
        for s in 0..si {
            let a = r * stride + s;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            m.triangle(a, c, b);
            m.triangle(b, c, d);
        }
    }
    m
}

/// Walec (boki + dwie pokrywy) o osi Y.
///
/// Dla kolumn, rur i beczułek. `sides` to liczba ścian bocznych —
/// 12 wystarczy, żeby przy tej skali nie było widocznych krawędzi.
pub fn cylinder(center: Vec3, radius: f32, half_height: f32, sides: u32, color: [f32; 3]) -> Mesh {
    let mut m = Mesh::new();
    let n = sides.max(3);
    let h = half_height;

    // --- boki
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();
        let i0 = m.vertices.len() as u32;
        // Normalna ściany bocznej to kierunek radialny w połowie łuku
        // — dzięki temu ściana nie ma widocznego załamania w miejscu
        // styku sąsiednich segmentów.
        let mid = (a0 + a1) * 0.5;
        let nrm = Vec3::new(mid.cos(), 0.0, mid.sin());
        m.push(Vertex::new_uv(
            center + Vec3::new(c0 * radius, -h, s0 * radius),
            nrm,
            [i as f32 / n as f32, 0.0],
            color,
        ));
        m.push(Vertex::new_uv(
            center + Vec3::new(c1 * radius, -h, s1 * radius),
            nrm,
            [(i + 1) as f32 / n as f32, 0.0],
            color,
        ));
        m.push(Vertex::new_uv(
            center + Vec3::new(c1 * radius, h, s1 * radius),
            nrm,
            [(i + 1) as f32 / n as f32, 1.0],
            color,
        ));
        m.push(Vertex::new_uv(
            center + Vec3::new(c0 * radius, h, s0 * radius),
            nrm,
            [i as f32 / n as f32, 1.0],
            color,
        ));
        m.triangle(i0, i0 + 1, i0 + 2);
        m.triangle(i0, i0 + 2, i0 + 3);
    }

    // --- pokrywy
    //
    // Dwie osobne pętle, bo poprawny wiatr wymaga odwrotnej kolejności
    // na górze i na dole. Próba współdzielenia pętli z `if` wygląda
    // krócej, ale przy debugowaniu cullinga jest nieczytelna.
    for (y, nrm, forward) in [(h, Vec3::Y, true), (-h, Vec3::NEG_Y, false)] {
        let c_idx = m.vertices.len() as u32;
        m.push(Vertex::new_uv(
            center + Vec3::new(0.0, y, 0.0),
            nrm,
            [0.5, 0.5],
            color,
        ));
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            m.push(Vertex::new_uv(
                center + Vec3::new(c * radius, y, s * radius),
                nrm,
                [c * 0.5 + 0.5, s * 0.5 + 0.5],
                color,
            ));
        }
        for i in 0..n {
            let a = c_idx + 1 + i;
            let b = c_idx + 1 + (i + 1) % n;
            if forward {
                m.triangle(c_idx, a, b);
            } else {
                m.triangle(c_idx, b, a);
            }
        }
    }
    m
}

/// Płaska płyta z podziałem na segmenty — podłoga i ściany.
///
/// Podział jest potrzebny, żeby cień i odbicia SSR miały co liczyć:
/// jedna wielka płyta dałaby płaszczyznę o stałej normalnej. Pozwala
/// też zafalować podłogę bez dokładania osobnych siatek.
pub fn plate(
    center: Vec3,
    half: Vec3,
    seg_x: u32,
    seg_z: u32,
    color_fn: impl Fn(f32, f32) -> [f32; 3],
) -> Mesh {
    let mut m = Mesh::new();
    let (nx, nz) = (seg_x.max(1), seg_z.max(1));

    for gz in 0..nz {
        for gx in 0..nx {
            let x0 = center.x - half.x + gx as f32 * (half.x * 2.0 / nx as f32);
            let x1 = center.x - half.x + (gx + 1) as f32 * (half.x * 2.0 / nx as f32);
            let z0 = center.z - half.z + gz as f32 * (half.z * 2.0 / nz as f32);
            let z1 = center.z - half.z + (gz + 1) as f32 * (half.z * 2.0 / nz as f32);
            let col = color_fn((x0 + x1) * 0.5, (z0 + z1) * 0.5);

            let p00 = Vec3::new(x0, center.y, z0);
            let p10 = Vec3::new(x1, center.y, z0);
            let p11 = Vec3::new(x1, center.y, z1);
            let p01 = Vec3::new(x0, center.y, z1);
            // Naprzemienne przekątne co wiersz — inaczej siatka daje
            // widoczny wzór schodków pod światłem.
            if (gx + gz) % 2 == 0 {
                m.tri(
                    Vertex::new(p00, Vec3::Y, col),
                    Vertex::new(p11, Vec3::Y, col),
                    Vertex::new(p10, Vec3::Y, col),
                );
                m.tri(
                    Vertex::new(p00, Vec3::Y, col),
                    Vertex::new(p01, Vec3::Y, col),
                    Vertex::new(p11, Vec3::Y, col),
                );
            } else {
                m.tri(
                    Vertex::new(p00, Vec3::Y, col),
                    Vertex::new(p01, Vec3::Y, col),
                    Vertex::new(p10, Vec3::Y, col),
                );
                m.tri(
                    Vertex::new(p10, Vec3::Y, col),
                    Vertex::new(p01, Vec3::Y, col),
                    Vertex::new(p11, Vec3::Y, col),
                );
            }
        }
    }
    m
}

/// Rura (cylindryczna) między dwoma punktami — przewody i kable.
///
/// Osobno od `cylinder`, bo rura bywa długa, cienka i idzie w dowolnym
/// kierunku. Budujemy ją z dwóch wektorów prostopadłych do osi, zamiast
/// z pełnej macierzy obrotu — to dwa mnożenia zamiast dziewięciu.
pub fn pipe(from: Vec3, to: Vec3, radius: f32, color: [f32; 3], sides: u32) -> Mesh {
    let dir = to - from;
    let len = dir.length();
    if len < 1e-5 {
        // Zerowa długość dałaby `normalize(0)` = NaN, a NaN w pozycji
        // wierzchołka znika w cullingu. Pusta siatka jest bezpieczna.
        return Mesh::new();
    }
    let axis = dir / len;
    // Gdy rura idzie pionowo, `cross` z osią Y zeruje się — wybieramy
    // wtedy inną oś pomocniczą.
    let helper = if axis.dot(Vec3::Y).abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let side = axis.cross(helper).normalize_or_zero();
    let other = axis.cross(side).normalize_or_zero();

    let mut m = Mesh::new();
    let n = sides.max(3);
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let d0 = side * a0.cos() + other * a0.sin();
        let d1 = side * a1.cos() + other * a1.sin();
        let mid = (a0 + a1) * 0.5;
        let nrm = side * mid.cos() + other * mid.sin();
        let i0 = m.vertices.len() as u32;
        m.push(Vertex::new_uv(
            from + d0 * radius,
            nrm,
            [i as f32 / n as f32, 0.0],
            color,
        ));
        m.push(Vertex::new_uv(
            to + d0 * radius,
            nrm,
            [i as f32 / n as f32, 1.0],
            color,
        ));
        m.push(Vertex::new_uv(
            to + d1 * radius,
            nrm,
            [(i + 1) as f32 / n as f32, 1.0],
            color,
        ));
        m.push(Vertex::new_uv(
            from + d1 * radius,
            nrm,
            [(i + 1) as f32 / n as f32, 0.0],
            color,
        ));
        m.triangle(i0, i0 + 2, i0 + 1);
        m.triangle(i0, i0 + 3, i0 + 2);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Normalna trójkąta `a,b,c` z mnożenia wektorów.
    fn tri_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
        (b - a).cross(c - a).normalize_or_zero()
    }

    /// Wszystkie ściany prostopadłościanu zwracają się na zewnątrz.
    ///
    /// To najczęstszy błąd tej warstwy i najdroższy w naprawie:
    /// trójkąt z podwórną orientacją znika po cichu, a objaw
    /// („dziury w podłodze") wskazuje zupełnie inne miejsce.
    #[test]
    fn box_faces_point_outwards() {
        let m = box_mesh(Vec3::ZERO, Vec3::splat(1.0), [1.0; 3]);
        for (i, ind) in m.indices.chunks(3).enumerate() {
            let a = m.vertices[ind[0] as usize].pos();
            let b = m.vertices[ind[1] as usize].pos();
            let c = m.vertices[ind[2] as usize].pos();
            let n = tri_normal(a, b, c);
            let centroid = (a + b + c) / 3.0;
            assert!(
                centroid.dot(n) > 0.0,
                "trójkąt {i} zwrócony do wewnątrz: n={n:?}, centroid={centroid:?}"
            );
        }
    }

    #[test]
    fn sphere_vertices_lie_on_the_sphere() {
        let c = Vec3::new(1.0, 2.0, 3.0);
        let r = 0.5;
        let m = sphere(c, r, 8, 12, [1.0; 3]);
        for v in &m.vertices {
            let dir = v.pos() - c;
            assert!(
                (dir.length() - r).abs() < 0.01,
                "wierzchołek {} leży {} od środka, a nie {r}",
                v.pos(),
                dir.length()
            );
            assert!(
                dir.normalize_or_zero().dot(v.normal()) > 0.99,
                "normalna {:?} nie zgadza się z pozycją {:?}",
                v.normal(),
                v.pos()
            );
        }
    }

    /// Ściany boczne walca mają normalne radialne.
    ///
    /// Uwaga na kryterium: JEDEN segment ma jedną normalną, wypośrodkowaną
    /// na kącie środkowym, a nie cztery różne. Dzięki temu walec ma
    /// płaskie „boki" jak cylinder, a nie gładki walec. Test porównuje
    /// więc normalną z KIERUNKIEM radialnym, sprawdzając znak iloczynu,
    /// a nie równość — równość wymagałaby czterech normalnych na
    /// segment, czyli gładkiego cieniowania, a to zabija krawędzie, o
    /// których tu chodzi.
    #[test]
    fn cylinder_side_normals_are_radial() {
        let m = cylinder(Vec3::ZERO, 1.0, 2.0, 8, [1.0; 3]);
        // 8 segmentów × 4 wierzchołki = 32 wierzchołki boków.
        for v in m.vertices.iter().take(32) {
            assert!(
                v.normal().y.abs() < 1e-5,
                "boczna normalna {:?} ma składową Y",
                v.normal()
            );
            let radial = Vec3::new(v.pos().x, 0.0, v.pos().z).normalize_or_zero();
            // Dodatni iloczyn = normalna wskazuje NA ZEWNĄTRZ. Dokładna
            // równość nie zachodzi dla wierzchołków narożnych segmentu,
            // bo ich normalna celuje w środek łuku.
            assert!(
                radial.dot(v.normal()) > 0.7,
                "normalna {:?} nie wskazuje na zewnątrz w punkcie {:?}",
                v.normal(),
                v.pos()
            );
        }
    }

    #[test]
    fn pipe_normals_are_perpendicular_to_axis() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(0.0, 5.0, 0.0);
        let m = pipe(a, b, 0.3, [1.0; 3], 8);
        assert!(!m.vertices.is_empty(), "rura jest pusta");
        let axis = (b - a).normalize_or_zero();
        for v in &m.vertices {
            let dot = v.normal().dot(axis);
            assert!(
                dot.abs() < 1e-3,
                "normalna {:?} nie jest prostopadła do osi (dot={dot})",
                v.normal()
            );
        }
    }

    #[test]
    fn plate_is_flat_and_upwards() {
        let m = plate(Vec3::ZERO, Vec3::new(4.0, 0.0, 4.0), 4, 4, |_, _| [1.0; 3]);
        assert_eq!(
            m.vertices.len(),
            4 * 4 * 6,
            "płyta ma nieoczekiwaną liczbę wierzchołków"
        );
        for v in &m.vertices {
            assert_eq!(v.normal(), Vec3::Y, "płyta nie jest pozioma");
            assert!(v.pos().y.abs() < 1e-6, "płyta ma nierówną wysokość");
        }
    }

    /// Rura o zerowej długości musi być pusta \u2014 inaczej
    /// `normalize(0)` daje NaN i cała bryła znika.
    #[test]
    fn degenerate_pipe_is_empty() {
        let m = pipe(Vec3::ZERO, Vec3::ZERO, 0.1, [1.0; 3], 8);
        assert!(m.vertices.is_empty(), "zerowa długość dała bryłę");
    }
}
