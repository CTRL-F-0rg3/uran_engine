//! Scena: industrialna platforma badawcza pod otwartym niebem.
//!
//! # Dlaczego ta scena
//!
//! Żeby ocenić styl Endfield, potrzeba otoczenia, które daje materiałowi
//! czym oddychać. Sama postać na pustej płaszczyźnie pokaże rysunkową
//! rampę, ale nie pokaże odbic SSR, mgły ani promieni słonecznych.
//!
//! Scena jest więc celowo zbudowana tak, żeby każdy z tych efektów
//! miał gdzie działać:
//!
//! | element sceny | co pokazuje |
//! |---|---|
//! | wypolerowana podłoga z panelami | odbicia SSR, anamorficzna poświata |
//! | słupy i belki | kontury ekranowe na krawędziach |
//! | neonowe listwy | bloom, flara, promienie słoneczne |
//! | otwarta przestrzeń nad platformą | fizyczne niebo Rayleigh + Mie |
//! | dystans do przeciwległej krawędzi | mgła eksponencjalna, DoF |
//! | jasny żółty rdzeń reaktora | ciepły odbłysk na otoczeniu |

use uran_math::Vec3;
use uran_render3d::Mesh;

use crate::geometry;

/// Połowa boku platformy. Scena jest kwadratowa.
pub const ARENA_HALF: f32 = 26.0;

/// Wysokość podłogi. Wszystko inne pozycjonujemy względem niej.
///
/// Osobna stała, a nie `0.0` w kodzie, bo podłoga jest PUNKTEM
/// odniesienia dla całej sceny: słupy, postacie i pociski liczą
/// wysokość od niej. Podniesienie podłogi o 2 m przesunęłoby wtedy
/// reaktor, a nie całą scenę razem z nim.
pub const FLOOR_Y: f32 = 0.0;

/// Paleta barw — spójna w całej scenie.
///
/// Trzymana w jednym miejscu, żeby zmiana motywu nie wymagała
/// grzebania w kilku plikach. Kolory są podane w sRGB, bo tak je
/// wpisuje się w kodzie i tak wyglądają w edytorze; shader przelicza
/// je na liniowe przed oświetleniem.
///
/// # UWAGA: to są kolory MATERIAŁÓW, nie wierzchołków
///
/// Domyślnie cała geometria w tym demo ma białe wierzchołki, a kolor
/// trafia do shadera przez `Surface::base` (patrz `build_materials`
/// w `main.rs`). JEDYNY wyjątek to podłoga, która świadomie używa
/// kolorów wierzchołków (`vertex_color: true`) — bo szew i gradient
/// muszą zmieniać się na jednej bryle.
///
/// Trzymanie jednego źródła prawdy dla większości siatek jest warte
/// tej dyscypliny: mieszanie obu ścieżek daje kolor, którego nie
/// widać w kodzie.
pub mod palette {
    /// Podłoga: ciemny, lekko ciepły beton.
    ///
    /// Niska chropowatość (patrz `build_materials`) daje tu odbicia —
    /// to ona odbija postać i listwy. Zbyt jasny kolor zabiłby
    /// odbicie, bo w PBR odbicie miesza się z bazą dyfuzyjną.
    pub const FLOOR: [f32; 3] = [0.26, 0.27, 0.30];
    /// Jasny pas co N segmentów — „szew" między panelami.
    pub const FLOOR_SEAM: [f32; 3] = [0.15, 0.16, 0.19];
    /// Ciemna metaliczna konstrukcja.
    pub const STRUCTURE: [f32; 3] = [0.32, 0.34, 0.40];
    /// Skrzynia transportowa — cieplejszy od konstrukcji.
    pub const CRATE: [f32; 3] = [0.44, 0.36, 0.28];
    /// Ciepły rdzeń reaktora — celowy wyjątek w chłodnej palecie.
    ///
    /// Ten żółty kolor ma konkretne zadanie: jest jasnym źródłem, które
    /// odbija się od sąsiednich powierzchni i dzięki temu widać, że
    /// postać NIE jest wycięta z tła.
    pub const CORE: [f32; 3] = [1.0, 0.78, 0.32];
    /// Emisja listew: chłodny cyjan, kontrast do ciepłego rdzenia.
    pub const NEON_COOL: [f32; 3] = [0.30, 0.85, 1.0];
    /// Emisja drugiego obiegu — ciepły bursztyn.
    pub const NEON_WARM: [f32; 3] = [1.0, 0.55, 0.25];
    /// Czerwone światło ostrzegawcze.
    pub const ALERT: [f32; 3] = [1.0, 0.24, 0.20];
    /// Skóra postaci.
    pub const SKIN: [f32; 3] = [0.96, 0.84, 0.78];
    /// Włosy — ciemne, żeby kontrastowały ze skórą.
    pub const HAIR: [f32; 3] = [0.16, 0.14, 0.20];
    /// Jaśniejsze pasmo włosów (grzywa, kuc) — łapie światło.
    pub const HAIR_BRIGHT: [f32; 3] = [0.34, 0.30, 0.42];
    /// Skafander: ciemny granatowy.
    pub const SUIT: [f32; 3] = [0.16, 0.20, 0.30];
    /// Ciemniejszy odcień skafandra (pas, rękawice, buty).
    pub const SUIT_DARK: [f32; 3] = [0.09, 0.11, 0.17];
    /// Klamra metalowa.
    pub const SUIT_BUCKLE: [f32; 3] = [0.55, 0.60, 0.68];
    /// Emisja wizjera.
    pub const VISOR: [f32; 3] = [0.35, 0.90, 1.0];
}

/// Kolor wierzchołka dla geometrii sterowanej materiałem: biel.
///
/// Kolor trafia do shadera przez `Surface::base`, więc wierzchołek nie
/// niesie barwy. Stała istnieje po to, żeby w każdym miejscu
/// budowania geometrii było WIDAĆ, że kolor pochodzi z materiału —
/// i żeby zmiana tej decyzji była jedną edycją.
pub const VERTEX_WHITE: [f32; 3] = [1.0, 1.0, 1.0];

/// Zestaw siatek sceny — rejestrowane raz przy starcie.
pub struct SceneMeshes {
    /// Podłoga z panelami (podzielona na segmenty dla cieni i SSR).
    pub floor: Mesh,
    /// Podest po centralnym pierścieniu.
    pub dais: Mesh,
    /// Rdzeń reaktora: kula wewnętrzna (świeci).
    pub core_inner: Mesh,
    /// Rdzeń: obudowa zewnętrzna (metal, łapie światło z rdzenia).
    pub core_shell: Mesh,
    /// Słup nośny.
    pub pillar: Mesh,
    /// Belka stropowa.
    pub beam: Mesh,
    /// Skrzynia transportowa.
    pub crate_mesh: Mesh,
    /// Bariera / listwa emisyjna.
    pub strip: Mesh,
    /// Rura przewodu.
    pub conduit: Mesh,
    /// Bryła przeciwnika: rdzeń.
    pub drone_core: Mesh,
    /// Bryła przeciwnika: skorupa.
    pub drone_shell: Mesh,
    /// Pocisk — mała, jasna bryła dobrze widoczna w ruchu.
    pub bolt: Mesh,
}

/// Czy (x, z) leży na linii szwu paneli.
///
/// Szwy to nie tekstura, tylko kolor wierzchołków, więc pozycje
/// siatki dzielimy tu. Dzięki temu podłoga ma czytelną siatkę
/// perspektywiczną, a nie jednolitą plamę — i jest coś, czego
/// oczy się złapią przy ruchu kamery.
fn on_seam(x: f32, z: f32) -> bool {
    const PANEL: f32 = 4.0;
    let fx = (x / PANEL).fract().abs();
    let fz = (z / PANEL).fract().abs();
    // 0.06 to szerokość szwu w jednostkach świata. Węższy znika
    // w aliasingu, szerszy wygląda jak mapa drogowa.
    fx < 0.06 || fz < 0.06
}

/// Buduje podłogę platformy.
///
/// Podział na 40×40 segmentów daje wystarczająco gęstą siatkę:
/// mapa cieni ma 2048² na całą scenę, więc jedna kratka podłogi
/// dostaje kilka texeli. Rzadsza siatka daje widoczne „schodki"
/// na granicach cienia.
pub fn build_floor() -> Mesh {
    geometry::plate(
        Vec3::new(0.0, FLOOR_Y, 0.0),
        Vec3::new(ARENA_HALF, 0.0, ARENA_HALF),
        40,
        40,
        |x, z| {
            // Podłoga JEST wyjątkiem: szew i gradient muszą zmieniać się
            // na jednej bryle, więc tu kolor siedzi w wierzchołkach
            // (materiał podłogi ma `vertex_color: true`).
            if on_seam(x, z) {
                palette::FLOOR_SEAM
            } else {
                // Delikatne zaciemnienie do środka platformy: środek
                // jest dalej od krawędzi, więc traci więcej światła
                // otoczenia. Daje wrażenie głębi bez geometrii.
                let d = (x * x + z * z).sqrt() / ARENA_HALF;
                let k = 1.0 - 0.18 * d;
                [
                    palette::FLOOR[0] * k,
                    palette::FLOOR[1] * k,
                    palette::FLOOR[2] * k,
                ]
            }
        },
    )
}

/// Wkłada bryłę `src` do `dst`, przesuwając ją do `at` i obracając
/// wokół osi Y o `yaw`.
///
/// Robimy to przez transformację pozycji i normalnych — ten sam
/// wzorzec, którego wymaga wczytanie `.obj`, tyle że napisany
/// ręcznie. Alternatywą była macierz, ale dla kilkunastu żeber
/// płacimy tylko za jedno mnożenie.
fn merge_into(dst: &mut Mesh, src: &Mesh, at: Vec3, yaw: f32) {
    let (s, c) = yaw.sin_cos();
    let rot = |p: Vec3| Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
    let base = dst.vertices.len() as u32;
    for v in &src.vertices {
        dst.vertices.push(uran_render3d::Vertex::new_uv(
            at + rot(v.pos()),
            rot(v.normal()),
            v.uv,
            v.color,
        ));
    }
    for idx in &src.indices {
        dst.indices.push(idx + base);
    }
}

/// Buduje wszystkie siatki sceny.
pub fn build_meshes() -> SceneMeshes {
    use geometry::{box_mesh, cylinder, pipe, sphere};
    // Kolor wierzchołka dla całej geometrii poza podłogą: biel, bo
    // barwę niesie materiał (`Surface::base`).
    let w = VERTEX_WHITE;

    // Podest centralny: niski walec pod reaktorem.
    let dais = cylinder(Vec3::new(0.0, 0.25, 0.0), 7.0, 0.25, 24, w);

    // --- rdzeń: świecąca kula w metalowej obudowie
    //
    // Dwie bryły, nie jedna: świecąca kula jest WIDOCZNA przez
    // szczeliny obudowy. Dlatego obudowa ma 8 pionowych żeber
    // zamiast pełnego cylindra — światło rdzenia wychodzi
    // między nimi i daje scenie główne źródło poświaty.
    let core_inner = sphere(Vec3::new(0.0, 3.4, 0.0), 1.5, 20, 24, w);

    // Żebra obudowy: 8 pionowych słupków wokół rdzenia.
    let mut core_shell = Mesh::new();
    for i in 0..8 {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        let pos = Vec3::new(c * 1.9, 2.4, s * 1.9);
        // Bryła jest budowana W ZERZE, a `pos` ją tylko ustawia —
        // inaczej przesunięcie byłoby policzone dwa razy.
        let rib = box_mesh(Vec3::ZERO, Vec3::new(0.16, 2.4, 0.30), w);
        merge_into(&mut core_shell, &rib, pos, a);
    }
    // Pierścienie na górze i na dole — spinają żebra w jedną bryłę.
    for y in [0.3f32, 4.6] {
        let ring = cylinder(Vec3::new(0.0, y, 0.0), 2.1, 0.14, 16, w);
        merge_into(&mut core_shell, &ring, Vec3::ZERO, 0.0);
    }

    // Słup: kwadratowy przekrój ostrzejszy niż walec.
    //
    // Kwadrat daje czytelne narożniki dla konturów ekranowych —
    // okrągły słup dałby krawędź, której głębokość zmienia się
    // płynnie i kontur by pulsował.
    let pillar = box_mesh(Vec3::ZERO, Vec3::new(0.55, 4.0, 0.55), w);
    // Belka stropowa dłuższa — jedna siatka, inna skala przy rysowaniu.
    let beam = box_mesh(Vec3::ZERO, Vec3::new(9.0, 0.35, 0.45), w);
    let crate_mesh = box_mesh(Vec3::ZERO, Vec3::splat(1.0), w);
    // Listwa emisyjna: wąski, długi prostopadłościan.
    let strip = box_mesh(Vec3::ZERO, Vec3::new(0.14, 0.05, 4.0), w);
    // Rura biegnie wzdłuż osi X.
    let conduit = pipe(
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        0.16,
        w,
        10,
    );

    // --- przeciwnik: skorupa + rdzeń
    //
    // Skorupa to ośmiokąt „pancerza", a rdzeń wystaje spomiędzy
    // segmentów i świeci — czytelny cel dla gracza.
    let drone_core = sphere(Vec3::ZERO, 0.42, 12, 16, w);
    let mut drone_shell = Mesh::new();
    for i in 0..8 {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        let pos = Vec3::new(a.cos() * 0.62, 0.0, a.sin() * 0.62);
        // Jak wyżej: geometria w zerze, pozycję dokłada `merge_into`.
        let plate_mesh = box_mesh(Vec3::ZERO, Vec3::new(0.10, 0.34, 0.20), w);
        merge_into(&mut drone_shell, &plate_mesh, pos, a);
    }

    // Pocisk: wydłużona bryła. Wydłużenie sprawia, że przy ruchu
    // widać kierunek lotu — kulka dawałaby kropkę bez informacji.
    let bolt = box_mesh(Vec3::ZERO, Vec3::new(0.10, 0.10, 0.55), w);

    SceneMeshes {
        floor: build_floor(),
        dais,
        core_inner,
        core_shell,
        pillar,
        beam,
        crate_mesh,
        strip,
        conduit,
        drone_core,
        drone_shell,
        bolt,
    }
}
