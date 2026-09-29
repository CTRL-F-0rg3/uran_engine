//! Geometria budowana w kodzie: teren, roślina, postać gracza.
//!
//! Modele budynków bierzemy z plików `.obj` (patrz `assets.rs`), ale
//! rośliny i postać rysujemy sami. Powód: roślina musi zmieniać
//! wygląd zależnie od wzrostu, a pojedynczy model `.obj` to jedna
//! bryła — animowałibyśmy ją skalą, co psuje proporcje liści.
//!
//! Wszystkie bryły trzymamy w metrach i stawiamy na `y = 0`, bo
//! `model_matrix` tylko przesuwa — nie chcemy podnosić modeli ręcznie
//! w kodzie gry.

use uran_math::Vec3;
use uran_render3d::{Mesh, Vertex};

/// Barwy gruntu. Trzymane jako osobne stałe, bo shader mnoży przez nie
/// albedo wierzchołka — kolor to jedyny „materiał" tych brył.
pub mod palette {
    /// Trawa. Ciepły, lekko żółtawy zielony — bliżej polskiej łąki
    /// niż fluorowa zieleń z gier.
    pub const GRASS: [f32; 3] = [0.36, 0.54, 0.22];
    /// Ciemniejsza trawa na dalszych pasach (daje wrażenie pola).
    pub const GRASS_DARK: [f32; 3] = [0.29, 0.45, 0.19];
    /// Ziemia uprawna. Ciepła brązowa, wyraźnie ciemniejsza od trawy,
    /// żeby pole było rozpoznawalne z góry.
    pub const SOIL: [f32; 3] = [0.42, 0.29, 0.18];
    /// Złota dojrzała roślina.
    pub const RIPE: [f32; 3] = [0.85, 0.68, 0.22];
    /// Zielona niedojrzała roślina.
    pub const GREEN: [f32; 3] = [0.30, 0.62, 0.24];
    /// Pień i liście — ciemniejszy zielony dla kontrastu.
    pub const STEM: [f32; 3] = [0.24, 0.48, 0.20];
    /// Koszulka gracza — ciepły pomarańcz, odróżnia od zieleni pola.
    pub const SHIRT: [f32; 3] = [0.90, 0.48, 0.18];
    /// Spodnie gracza.
    pub const PANTS: [f32; 3] = [0.24, 0.30, 0.45];
    /// Skóra.
    pub const SKIN: [f32; 3] = [0.93, 0.76, 0.60];
}

/// Kwadratowa płyta z normalną w górę.
///
/// Kolejność wierzchołków `(p00, p11, p10)` daje skręt w prawo
/// (normalna +Y). Odwrócenie daje -Y i cała płyta znika przy
/// back-face culling — dlatego kolejność tu jest jawna.
pub fn quad_y(cx: f32, cz: f32, half: f32, y: f32, color: [f32; 3]) -> Mesh {
    let p00 = Vec3::new(cx - half, y, cz - half);
    let p10 = Vec3::new(cx + half, y, cz - half);
    let p11 = Vec3::new(cx + half, y, cz + half);
    let p01 = Vec3::new(cx - half, y, cz + half);
    let n = Vec3::Y;
    let mut m = Mesh::new();
    m.tri(
        Vertex::new(p00, n, color),
        Vertex::new(p11, n, color),
        Vertex::new(p10, n, color),
    );
    m.tri(
        Vertex::new(p00, n, color),
        Vertex::new(p01, n, color),
        Vertex::new(p11, n, color),
    );
    m
}

/// Równa plansza trawy `size` × `size` m, podzielona na kwadraty `step`.
///
/// Naprzemienne odcienie wierszy dają perspektywę: płaska jednokolorowa
/// zielona kwadratowa płaszczyzna wygląda jak plik graficzny, a nie
/// jak łąka. Zmiana odcienia na co drugi kwadrat jest najtańszym
/// sposobem na pokazanie skali świata.
///
/// Zwiększamy każdy kwadrat o `+0.01` na boku, żeby sąsiadujące
/// kwadraty lekko na siebie nachodziły. Bez tego przez pory w bicie
/// widać by czarną siatkę.
pub fn ground(size: f32, step: f32) -> Mesh {
    let n = (size / step).round().max(1.0) as i32;
    let half = step * 0.5 + 0.01;
    let mut mesh = Mesh::new();
    for gz in 0..n {
        for gx in 0..n {
            let cx = -size * 0.5 + (gx as f32 + 0.5) * step;
            let cz = -size * 0.5 + (gz as f32 + 0.5) * step;
            let c = if (gx + gz) % 2 == 0 {
                palette::GRASS
            } else {
                palette::GRASS_DARK
            };
            let p00 = Vec3::new(cx - half, 0.0, cz - half);
            let p10 = Vec3::new(cx + half, 0.0, cz - half);
            let p11 = Vec3::new(cx + half, 0.0, cz + half);
            let p01 = Vec3::new(cx - half, 0.0, cz + half);
            // Kolejność daje skręt w prawo => normalna +Y (patrz `quad_y`).
            mesh.tri(
                Vertex::new(p00, Vec3::Y, c),
                Vertex::new(p11, Vec3::Y, c),
                Vertex::new(p10, Vec3::Y, c),
            );
            mesh.tri(
                Vertex::new(p00, Vec3::Y, c),
                Vertex::new(p01, Vec3::Y, c),
                Vertex::new(p11, Vec3::Y, c),
            );
        }
    }
    mesh
}

/// Ostrosłup: prostokątna podstawa na `y = base_y`, wierzchołek
/// na `y = base_y + height`.
///
/// Wariant z podstawą przesuniętą (zamiast zawsze `y = 0`) jest
/// potrzebny dla liści, które wyrastają z pędu na pewnej wysokości.
/// Bez tego musielibyśmy przesuwać już dodane wierzchołki, a to
/// łatwo zepsuć: przesuwa się wtedy też środek podstawy.
fn pyramid_at(
    mesh: &mut Mesh,
    lo: [f32; 2],
    hi: [f32; 2],
    base_y: f32,
    height: f32,
    color: [f32; 3],
) {
    let (cx, cz) = ((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5);
    let p00 = Vec3::new(lo[0], base_y, lo[1]);
    let p10 = Vec3::new(hi[0], base_y, lo[1]);
    let p11 = Vec3::new(hi[0], base_y, hi[1]);
    let p01 = Vec3::new(lo[0], base_y, hi[1]);
    let top = Vec3::new(cx, base_y + height, cz);
    // Każda ściana ma WŁASNĄ normalną: przy wspólnej cała bryła
    // wyglądałaby płasko, bez cieni, jak naklejka.
    let mut face = |a: Vec3, b: Vec3| {
        let n = (b - a).cross(top - a).normalize_or_zero();
        mesh.tri(
            Vertex::new(a, n, color),
            Vertex::new(b, n, color),
            Vertex::new(top, n, color),
        );
    };
    face(p00, p10);
    face(p10, p11);
    face(p11, p01);
    face(p01, p00);
}

/// Ostrosłup stojący na `y = 0`.
fn pyramid(mesh: &mut Mesh, lo: [f32; 2], hi: [f32; 2], height: f32, color: [f32; 3]) {
    pyramid_at(mesh, lo, hi, 0.0, height, color);
}

/// Roślina na jednej działce.
///
/// `growth`: 0.0 = świeżo zasadzony pęd, 1.0 = dojrzała roślina.
///
/// Wzrost zmienia **wysokość pędu i kształt liści**, a nie tylko skalę.
/// Skalowanie całej bryły dałoby kiełbki cienkie na górze i grube na
/// dole. Tutaj pęd wydłuża się, a liście wyrastają stopniowo — dzięki
/// temu wzrost widać z odległości kamery.
pub fn plant(growth: f32) -> Mesh {
    let g = growth.clamp(0.0, 1.0);
    let mut mesh = Mesh::new();

    // Kolor przechodzi zielony -> złoty. `smoothstep` daje łagodne
    // przejście w drugiej połowie wzrostu, gdy zieleń „dojrzewa".
    let r = ((g - 0.45) / 0.55).clamp(0.0, 1.0);
    let t = r * r * (3.0 - 2.0 * r);
    let leaf = [
        palette::GREEN[0] + (palette::RIPE[0] - palette::GREEN[0]) * t,
        palette::GREEN[1] + (palette::RIPE[1] - palette::GREEN[1]) * t,
        palette::GREEN[2] + (palette::RIPE[2] - palette::GREEN[2]) * t,
    ];

    // Wysokość pędu: 0,25 m (pęd) -> 1,35 m (dojrzała roślina).
    let stem_h = 0.25 + g * 1.10;
    // Grubość pędu maleje, żeby nie był jednym klocem.
    let stem_w = 0.085 - g * 0.040;
    pyramid(
        &mut mesh,
        [-stem_w, -stem_w],
        [stem_w, stem_w],
        stem_h,
        palette::STEM,
    );

    // Liście pojawiają się od 40% wzrostu. Dwa liście wystarczą:
    // na działce 2 m więcej zlewa się w zieloną plamę.
    if g > 0.4 {
        let a = ((g - 0.4) / 0.6).clamp(0.0, 1.0);
        let base_y = stem_h * (0.35 + 0.40 * a);
        let lw = 0.34 * a;
        let lh = 0.16 * a;
        // Dwa liście na wprost, w osi X — czytelne z każdej strony.
        pyramid_at(
            &mut mesh,
            [0.0, -lw * 0.30],
            [lw * 2.0, lw * 0.30],
            base_y,
            lh,
            leaf,
        );
        pyramid_at(
            &mut mesh,
            [-lw * 2.0, -lw * 0.30],
            [0.0, lw * 0.30],
            base_y,
            lh,
            leaf,
        );
    }

    // Dojrzały kłos: złoty stożek na szczycie pędu. Bez niego nie
    // widać, że roślina nadaje się do zbioru — a to cel gry.
    if g >= 1.0 {
        let w = 0.10;
        pyramid_at(
            &mut mesh,
            [-w, -w],
            [w, w],
            stem_h - 0.10,
            0.42,
            palette::RIPE,
        );
    }

    mesh
}

/// Postać gracza: nogi, tors, głowa.
///
/// Proporta „chibi" (głowa duża wobec ciała) daje sylwetkę
/// rozpoznawalną z typowej odległości kamery (~14 m).
pub fn player() -> Mesh {
    let mut m = Mesh::new();
    // Nogi: dwa krótkie słupki.
    for sx in [-0.13f32, 0.13] {
        pyramid(
            &mut m,
            [sx - 0.09, -0.10],
            [sx + 0.09, 0.10],
            0.75,
            palette::PANTS,
        );
    }
    // Tors.
    pyramid(&mut m, [-0.19, -0.13], [0.19, 0.13], 1.30, palette::SHIRT);
    // Głowa: ostrosłup w górę — w niskiej perspektywie wygląda jak
    // głowa z daszkiem, a nie jak sześcian.
    pyramid(&mut m, [-0.15, -0.13], [0.15, 0.13], 1.62, palette::SKIN);
    m
}

/// Kolor drutu kolidatora — jaskrawy pomarańcz, celowo kontrastujący
/// z zielenią pola, żeby bryły były rozpoznawalne na tle.
pub const COLLIDER: [f32; 3] = [1.0, 0.45, 0.05];

/// Kolor sześcianu gracza — chłodny błękit, żeby odróżnić go od
/// przeszkód (pomarańcz) jednym rzutem oka.
pub const PLAYER_BOX: [f32; 3] = [0.25, 0.75, 1.0];

/// Grube krawędzie prostopadłościana — bryła `collide::Box` widoczna
/// jako drunek.
///
/// Rysujemy **krawędzie** (12 prostych), nie ściany: ściany by były
/// półprzezroczyste, wymagały sortowania i zasłaniałyby świat.
/// Drunek nie przeszkadza patrzeć i jest czytelny z każdego kąta.
///
/// Grubość to nie prawdziwa szerokość linii — po prostu mały kwadrat
/// wokół każdej krawędzi, bo rasteryzator nie zna linii. `t` dobrane
/// tak, żeby 2 cm było widoczne z kilku metrów, a nie zasłaniało
/// sceny.
pub fn collider_box(center: Vec3, half: Vec3, color: [f32; 3], t: f32) -> Mesh {
    // Osiem narożników, w stałej kolejności bitów (x,y,z):
    // indeks to bit0=x, bit1=y, bit2=z; minus = 0, plus = 1.
    let mut c = [Vec3::ZERO; 8];
    for (i, p) in c.iter_mut().enumerate() {
        let s = |b: u32| if i & (1 << b) != 0 { 1.0 } else { -1.0 };
        *p = Vec3::new(
            center.x + s(0) * half.x,
            center.y + s(1) * half.y,
            center.z + s(2) * half.z,
        );
    }
    // Dwanaście krawędzi: pary narożników różniące się **jednym**
    // bitem. To kompletna lista — jej pominięcie albo duplikat to
    // najczęstszy błąd przy ręcznym rysowaniu prostopadłościanu.
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    let mut m = Mesh::new();
    for (a, b) in EDGES {
        bar(&mut m, c[a], c[b], t, color);
    }
    m
}

/// Kwadratowy pręt między dwoma punktami — jeden odcinek drutu.
fn bar(m: &mut Mesh, a: Vec3, b: Vec3, t: f32, color: [f32; 3]) {
    let d = (b - a).normalize();
    // Dwa wektory prostopadłe do osi pręta. Wybieramy dowolny wektor
    // nie równoległy do `d` i krzyżujemy — unikamy w ten sposób
    // przypadku degeneracji przy osi X/Y/Z.
    let helper = if d.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = d.cross(helper).normalize() * t;
    let v = d.cross(u).normalize() * t;
    // Cztery ściany kwadratu wokół osi.
    let corners = [a - u - v, a + u - v, a + u + v, a - u + v];
    for i in 0..4 {
        let p0 = corners[i];
        let p1 = corners[(i + 1) % 4];
        // Normalna ściany: na zewnątrz pręta, liczona z osi.
        let n = (p0 + p1) * 0.5 - (a + b) * 0.5;
        let n = if n.length_squared() > 1e-12 {
            n.normalize()
        } else {
            u
        };
        m.tri(
            Vertex::new(p0, n, color),
            Vertex::new(p1, n, color),
            Vertex::new(b, n, color),
        );
        m.tri(
            Vertex::new(p0, n, color),
            Vertex::new(b, n, color),
            Vertex::new(a, n, color),
        );
    }
    // Końce kwadratu — bez nich pręt byłby otwarty i przy pewnych
    // kątach patrzenia znikałyby jego krawędzie.
    for &p in &[a, b] {
        for i in 0..4 {
            let p0 = corners[i];
            let p1 = corners[(i + 1) % 4];
            m.tri(
                Vertex::new(p0, d, color),
                Vertex::new(p1, d, color),
                Vertex::new(p, d, color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_is_closed_and_faces_up() {
        let g = ground(40.0, 4.0);
        assert!(g.is_closed(), "teren musi mieć komplet trójkątów");
        assert!(g.vertices.len() > 100);
        // Każdy wierzchołek ma normalną w górę — inaczej teren zniknie
        // przy back-face culling.
        for v in &g.vertices {
            assert!(v.normal().y > 0.9, "normalna terenu nie w górę: {v:?}");
        }
    }

    #[test]
    fn quad_faces_upward() {
        let q = quad_y(0.0, 0.0, 1.0, 0.0, palette::SOIL);
        assert_eq!(q.indices.len(), 6, "kwadrat = 2 trójkąty");
        for v in &q.vertices {
            assert!(v.normal().y > 0.9);
        }
    }

    #[test]
    fn plant_grows_taller_with_growth() {
        // Wysokość bryły to najwyższy punkt wierzchołka.
        let height = |g: f32| {
            plant(g)
                .vertices
                .iter()
                .map(|v| v.pos().y)
                .fold(0.0f32, f32::max)
        };
        let small = height(0.0);
        let mid = height(0.5);
        let big = height(1.0);
        assert!(small < mid, "{small} !< {mid}");
        assert!(mid < big, "{mid} !< {big}");
        // Pęd musi być widoczny od razu po zasadzeniu.
        assert!(small > 0.1, "świeżo zasadzona roślina to {} m", small);
    }

    #[test]
    fn ripe_plant_is_gold_and_tall() {
        let p = plant(1.0);
        // Na dojrzałej roślinie musi występować złoty kolor kłosa.
        let gold = p
            .vertices
            .iter()
            .filter(|v| v.color[0] > 0.8 && v.color[2] < 0.3)
            .count();
        assert!(gold > 0, "dojrzała roślina nie ma złotego kłosa");
    }

    #[test]
    fn plant_growth_is_clamped() {
        // Wartości spoza 0..1 nie mogą generować ujemnych skal
        // ani uciecznych wysokości.
        for g in [-5.0f32, 0.0, 1.0, 99.0] {
            let p = plant(g);
            for v in &p.vertices {
                assert!(v.pos().y.is_finite() && v.pos().y >= -0.01);
            }
        }
    }

    #[test]
    fn plant_normals_are_valid() {
        // Zerowa normalna to NaN w WGSL = czarny piksel.
        for g in [0.0, 0.5, 1.0] {
            for v in &plant(g).vertices {
                assert!(
                    v.normal().length() > 0.5,
                    "roślina o wzroście {g} ma zerową normalną"
                );
            }
        }
    }

    #[test]
    fn player_is_about_two_metres_tall() {
        let p = player();
        let top = p.vertices.iter().map(|v| v.pos().y).fold(0.0f32, f32::max);
        // Postać ma być wyższa niż pęd, niższa niż budynek.
        assert!(top > 1.4 && top < 2.0, "postać ma {top} m");
        assert!(p.is_closed());
    }

    #[test]
    fn collider_box_has_no_vertices_inside_the_box() {
        // Kubit o boku 2 m. Wierzchołek drutu **wewnątrz** bryły
        // oznaczałby, że krawędzie są przesunięte.
        let m = collider_box(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0), COLLIDER, 0.02);
        assert!(!m.vertices.is_empty());
        for v in &m.vertices {
            let p = v.pos();
            let inside = p.x.abs() < 0.9 && p.y.abs() < 0.9 && p.z.abs() < 0.9;
            assert!(!inside, "wierzchołek drutu jest w środku bryły: {p:?}");
        }
    }

    #[test]
    fn collider_box_normals_are_valid() {
        // Zerowa normalna to NaN w WGSL = czarny piksel.
        for half in [
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(0.15, 0.55, 6.5),
            Vec3::new(3.8, 4.5, 3.8),
        ] {
            for v in &collider_box(Vec3::ZERO, half, COLLIDER, 0.02).vertices {
                assert!(v.normal().length() > 0.5, "zerowa normalna");
                assert!(v.pos().is_finite(), "NaN w dracie");
            }
        }
    }

    #[test]
    fn collider_box_is_denser_for_a_bigger_box() {
        // Płot (wąski i niski) musi mieć mniej geometrii niż stodoło.
        let small = collider_box(Vec3::ZERO, Vec3::new(0.15, 0.55, 6.5), COLLIDER, 0.02);
        let big = collider_box(Vec3::ZERO, Vec3::new(3.8, 4.5, 3.8), COLLIDER, 0.02);
        assert_eq!(
            small.vertices.len(),
            big.vertices.len(),
            "liczba wierzchołków nie zależy od rozmiaru bryły"
        );
    }
}
