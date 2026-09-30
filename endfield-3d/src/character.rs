//! Postać gracza: sylwetka, animacja i materiały.
//!
//! # Proportje
//!
//! Celowo stylizowane: głowa jest duża w stosunku do tułowia, kończyny
//! cienkie, barki wąskie. To proporcje „semi-realistic anime", w których
//! czytelna jest głowa z oblicza, a tułów niesie informację o ruchu.
//! Proporcje realistyczne wyglądałyby w kadrze jak sylwetka z daleka.
//!
//! # Materiały
//!
//! Tu widać całą hybrydę PBR + NPR naraz:
//!
//! | część | materiał | co pokazuje |
//! |---|---|---|
//! | skóra | `Surface::skin` | mocna rampa + SSS + ciepłe krawędzie |
//! | włosy | `Surface::hair` | anizotropia (pasmowy połysk) + rim |
//! | skafander | `Surface::cloth` | rysunkowa forma, niski połysk |
//! | klamry | `Surface::metal` | pełne PBR, niska stylizacja |
//! | wizjer | `Surface::emissive` | emisja HDR → bloom + anamorficzna poświata |
//!
//! Wszystkie materiały mają `stylize` różne, a scena ma własną skalę
//! (`Stylization::amount`). To one razem decydują, że postać jest
//! rysunkowa, a otoczenie fizyczne.

use uran_math::Vec3;
use uran_render3d::{Mesh, Vertex};

use crate::geometry;
use crate::world::palette;

/// Wysokość postaci w metrach.
///
/// 1.75 m to standard, ale przy tej skali otoczenia (26 m platforma)
/// sylwetka byłaby plamką. Trzymamy się realistycznej wysokości,
/// a czytelność zapewnia mocna rampa i kontur.
pub const HEIGHT: f32 = 1.75;

/// Identyfikatory materiałów postaci.
pub struct CharacterMaterials {
    pub skin: uran_render3d::MaterialId,
    pub hair: uran_render3d::MaterialId,
    /// Jaśniejsze pasmo włosów (grzywa, kuc).
    ///
    /// Osobny materiał, bo ma inny połysk. Gdyby grzywa i czupryna
    /// były jedną bryłą, dzieliłyby jeden materiał — a jasna grzywa
    /// na ciemnych włosach jest jednym z głównych akcentów rysunkowych
    /// sylwetki.
    pub hair_bright: uran_render3d::MaterialId,
    pub suit: uran_render3d::MaterialId,
    pub suit_dark: uran_render3d::MaterialId,
    pub metal: uran_render3d::MaterialId,
    pub visor: uran_render3d::MaterialId,
}

/// Siatki postaci — jedna na materiał, rysowane oddzielnie.
pub struct CharacterMeshes {
    /// Głowa (kula) + szyja.
    pub head: Mesh,
    /// Czupryna włosów (ciemna).
    pub hair: Mesh,
    /// Grzywa i kuc (jasne pasmo) — osobna siatka, bo osobny materiał.
    pub hair_bright: Mesh,
    /// Tułów skafandra.
    pub torso: Mesh,
    /// Biodra.
    pub hips: Mesh,
    /// Ramię (jedno, lustrzane w pozycji).
    pub arm: Mesh,
    /// Noga (jedna).
    pub leg: Mesh,
    /// But / stopa.
    pub boot: Mesh,
    /// Klamra metalowa na pasie.
    pub buckle: Mesh,
    /// Wzrok/visor na kasku.
    pub visor: Mesh,
}

/// Zestaw materiałów postaci.
///
/// Tworzymy je RAZ przy starcie i trzymamy w `Renderer3d::materials`,
/// bo `SceneProxy` rysuje z innego systemu, ale ten sam bank.
///
/// # Skąd bierze się kolor
///
/// Wszystkie materiały mają `vertex_color = false` (wartość
/// domyślna), więc kolor pochodzi z `Surface::base`, a geometria ma
/// białe wierzchołki. Dzięki temu zmiana odcienia skóry to jedna
/// edycja tego pliku, a nie przeklikanie przez wszystkie wywołania
/// `sphere(...)`.
pub fn build(
    bank: &mut uran_render3d::MaterialBank,
    gpu: &uran_render::GpuContext,
) -> CharacterMaterials {
    use uran_render3d::Surface;
    let mut s = |surf: Surface| bank.add_surface(&gpu.device, &gpu.queue, &surf);

    // Skóra: rampa 0.85 + SSS 1.0. `rim` 0.35 daje tę cienką
    // poświatę wokół sylwetki, która odcina postać od tła.
    let skin = s(Surface {
        rim: 0.35,
        ..Surface::skin(palette::SKIN)
    });

    // Włosy: anizotropia 0.85 rozciąga połysk wzdłuż włókna,
    // dzięki czemu błyszczą pasmem, a nie okręgiem.
    let hair = s(Surface {
        anisotropy: 0.85,
        ..Surface::hair(palette::HAIR)
    });
    let hair_bright = s(Surface {
        anisotropy: 0.7,
        ..Surface::hair(palette::HAIR_BRIGHT)
    });

    // Skafander: tkanina, rysunkowa, z lekkim SSS na krawędziach
    // (prześwitujący materiał).
    let suit = s(Surface::cloth(palette::SUIT));
    let suit_dark = s(Surface::cloth(palette::SUIT_DARK));

    // Klamry: pełne PBR bez rampy. Kontrast z rysunkowym skafandrem
    // jest tu zamierzony — to ten sam materiał co belki w otoczeniu.
    let metal = s(Surface::metal(palette::SUIT_BUCKLE, 0.28));

    // Visor: emisja 3.0 (HDR). Przekracza próg bloom, więc dostaje
    // poświatę, a anamorficzna poświata zrobi z niego poziomą smugę.
    let visor = s(Surface::emissive(palette::VISOR, 3.0));

    CharacterMaterials {
        skin,
        hair,
        hair_bright,
        suit,
        suit_dark,
        metal,
        visor,
    }
}

/// Wysokości stawów w skali postaci.
///
/// Procenty ciała dla postacji 1.75 m. Głowa zajmuje ~1/7.5, a nie
/// 1/8 — to stylizowany podział, który odróżnia anime od anatomii.
/// Trzymamy je w jednym miejscu, żeby zmiana `HEIGHT` była jedną
/// edycją i nic się nie rozjechało.
#[derive(Clone, Copy)]
pub struct Rig {
    pub head_y: f32,
    pub shoulder_y: f32,
    pub chest_y: f32,
    pub waist_y: f32,
    pub hip_y: f32,
    pub knee_y: f32,
    pub ankle_y: f32,
}

pub fn rig() -> Rig {
    let h = HEIGHT;
    Rig {
        head_y: h * 0.925,
        shoulder_y: h * 0.820,
        chest_y: h * 0.700,
        waist_y: h * 0.560,
        hip_y: h * 0.520,
        knee_y: h * 0.290,
        ankle_y: h * 0.060,
    }
}

/// Wkłada bryłę `src` do `dst` z przesunięciem do `at` i obrotem
/// `yaw` wokół osi Y.
fn merge_into(dst: &mut Mesh, src: &Mesh, at: Vec3, yaw: f32) {
    let (s, c) = yaw.sin_cos();
    let rot = |p: Vec3| Vec3::new(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
    let base = dst.vertices.len() as u32;
    for v in &src.vertices {
        dst.vertices.push(Vertex::new_uv(
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

/// Przesuwa całą siatkę o `offset`.
///
/// Osobna funkcja, bo przesuwanie bryły „w miejscu" wymagałoby
/// sklonowania jej i zapisu z powrotem (dwa użytki `&mut` naraz).
/// Normalnych nie ruszamy — przesunięcie nie zmienia kierunku
/// powierzchni.
fn translate(mesh: &mut Mesh, offset: Vec3) {
    for v in &mut mesh.vertices {
        *v = Vertex::new_uv(v.pos() + offset, v.normal(), v.uv, v.color);
    }
}

/// Buduje bryły postaci. Wszystko liczone w skali 1:1 z metrami.
pub fn build_meshes() -> CharacterMeshes {
    use geometry::{box_mesh, cylinder, sphere};
    let r = rig();

    // --- głowa: kula lekko wydłużona w osi Y
    //
    // `sphere` daje normalne wychodzące ze środka, więc oświetlenie
    // głowy jest gładkie — bez widocznych facetów. To ważne, bo głowa
    // jest najbliżej kamery i najbardziej rzuca się w oczy.
    let mut head = sphere(Vec3::ZERO, 0.112, 16, 20, crate::world::VERTEX_WHITE);
    for v in &mut head.vertices {
        let p = v.pos();
        *v = Vertex::new_uv(
            Vec3::new(p.x * 0.95, p.y * 1.18, p.z * 0.95),
            v.normal(),
            v.uv,
            v.color,
        );
    }
    // Szyja: krótki walec pod głową. Bez niej głowa „lewiłaby"
    // w powietrzu nad tułowiem przy ruchu.
    let neck = cylinder(Vec3::ZERO, 0.045, 0.075, 10, crate::world::VERTEX_WHITE);
    merge_into(&mut head, &neck, Vec3::new(0.0, r.head_y, 0.0), 0.0);

    // --- włosy
    //
    // Dwie siatki, nie jedna: jedna kula „przykryłaby" głowę z przodu
    // i nie było widać twarzy. Grzywa i kuc są osobno — to one dają
    // czytelny zarys włosów na obliczu, główny nośnik rysunkowego
    // stylu w Endfield. Osobna siatka, bo mają inny materiał.
    let w = crate::world::VERTEX_WHITE;
    let hair = sphere(Vec3::new(0.0, r.head_y + 0.014, -0.010), 0.121, 14, 18, w);

    // Jasne pasmo: grzywa z przodu + kuc z tyłu. Budujemy je przy
    // ORIGINIE i przesuwamy całość na głowę dopiero na końcu —
    // przesuwanie każdej bryły osobno dałoby dwie różne pozycje
    // do zapamiętania i łatwo by się rozjechały.
    let mut hair_bright = box_mesh(Vec3::ZERO, Vec3::new(0.150, 0.028, 0.030), w);
    let mut tail = sphere(Vec3::ZERO, 0.042, 8, 12, w);
    for v in &mut tail.vertices {
        let p = v.pos();
        *v = Vertex::new_uv(
            Vec3::new(p.x, p.y * 1.9, p.z * 0.8),
            v.normal(),
            v.uv,
            v.color,
        );
    }
    merge_into(&mut hair_bright, &tail, Vec3::new(0.0, -0.090, -0.110), 0.0);
    // Całość na głowę: grzywa z przodu, kuc z tyłu.
    translate(&mut hair_bright, Vec3::new(0.0, r.head_y + 0.060, 0.0));

    // --- tułów skafandra
    //
    // Prostopadłościan, do którego doklejamy zaokrąglone „kule"
    // ramion. Narożniki kwadratu dają konturowi ekranowemu twarde
    // załamania; same kule wyglądałyby jak lalka.
    let mut torso = box_mesh(
        Vec3::new(0.0, r.chest_y, 0.0),
        Vec3::new(0.230, 0.230, 0.130),
        crate::world::VERTEX_WHITE,
    );
    for side in [-1.0f32, 1.0] {
        let cap = sphere(Vec3::ZERO, 0.052, 8, 12, crate::world::VERTEX_WHITE);
        merge_into(
            &mut torso,
            &cap,
            Vec3::new(side * 0.108, r.shoulder_y, 0.0),
            0.0,
        );
    }
    // Pas: ciemna opaska oddzielająca tułów od bioder. Wzrok potrzebuje
    // poziomego podziału, inaczej sylwetka to jedna plama.
    let belt = box_mesh(
        Vec3::ZERO,
        Vec3::new(0.215, 0.022, 0.126),
        crate::world::VERTEX_WHITE,
    );
    merge_into(&mut torso, &belt, Vec3::new(0.0, r.waist_y, 0.0), 0.0);

    let hips = box_mesh(
        Vec3::new(0.0, r.hip_y, 0.0),
        Vec3::new(0.205, 0.090, 0.120),
        crate::world::VERTEX_WHITE,
    );

    // Klamra: metalowa blaszka z przodu pasa. Jedyny element postać
    // w pełnie metaliczny — łapie światło inaczej niż reszta.
    let buckle = box_mesh(
        Vec3::new(0.0, r.waist_y, 0.128),
        Vec3::new(0.062, 0.026, 0.016),
        crate::world::VERTEX_WHITE,
    );

    // --- kończyny
    //
    // Jedna siatka na ramię i jedna na nogę; drugą stronę odbijamy
    // skalą (-1) przy rysowaniu. Postać jest symetryczna za darmo,
    // a w pamięci jest o połowę mniej siatek.
    let arm_len = r.shoulder_y - r.waist_y;
    // Walec budujemy JEDNOSTKOWO: `cylinder` przyjmuje POŁOWĘ
    // wysokości, więc 1.0 daje oś Y w zakresie -1..1, licząc od
    // środka bryły. Poniżej przeliczamy ją na długość w metrach.
    let mut arm = cylinder(Vec3::ZERO, 0.042, 1.0, 8, crate::world::VERTEX_WHITE);
    for v in &mut arm.vertices {
        let p = v.pos();
        // Ściągamy ramię do elipsoidy: grubsze u barku, cieńsze przy
        // dłoni. Cylinder miałby jednakową grubość i wyglądał jak kij.
        //
        // `p.y` to -1..1, a ramię wisi W DÓŁ od barku: górna krawędź
        // walca (p.y = 1) to bark, więc dłoń jest na p.y = -1.
        //
        // `t` to 0 u barku i 1 przy dłoni. Pozycję Y liczymy jako
        // `-t * arm_len`, a nie `p.y * arm_len` — inaczej połowa
        // ramienia wystawałaby NAD barkiem, bo `p.y` zmienia znak.
        // Ściąganie `k` robimy w tej samej skali co długość, inaczej
        // ramię byłoby grubsze przy dłoni niż u barku.
        let t = (1.0 - p.y) * 0.5;
        let k = 1.0 - 0.35 * t;
        *v = Vertex::new_uv(
            Vec3::new(p.x * k, -t * arm_len, p.z * k),
            v.normal(),
            v.uv,
            v.color,
        );
    }
    // Rękawica na końcu ramienia: kontrastowa bryła.
    let glove = box_mesh(
        Vec3::ZERO,
        Vec3::new(0.036, 0.048, 0.036),
        crate::world::VERTEX_WHITE,
    );
    // Dłoń leży na p.y = -1, czyli `arm_len` metrów POD barkiem.
    // Umieszczając rękawicę na +arm_len, wisiałaby w powietrzu nad
    // postacią i nie pasowałaby do reszty ramienia.
    merge_into(&mut arm, &glove, Vec3::new(0.0, -arm_len, 0.0), 0.0);

    // Noga: udo + podudzie, dwie bryły o różnej grubości.
    //
    // Dwie uwagi do `cylinder`:
    //  1. przyjmuje POŁOWĘ wysokości, więc długość między stawami
    //     podajemy przez połowę — inaczej noga byłaby dwa razy za
    //     długa i stopy wpadłyby pod podłogę;
    //  2. bryła jest symetryczna wokół środka, a punkt zaczepienia
    //     (biodro) ma być w y = 0, więc u dodatkowo przesuwamy
    //     w dół o swoją połowę.
    let thigh = r.hip_y - r.knee_y;
    let shin = r.knee_y - r.ankle_y;
    let mut leg = cylinder(
        Vec3::new(0.0, -thigh * 0.5, 0.0),
        0.062,
        thigh * 0.5,
        8,
        crate::world::VERTEX_WHITE,
    );
    // Podudzie zaczyna się w kolanie, czyli o `thigh` niżej.
    let calf = cylinder(Vec3::ZERO, 0.048, shin * 0.5, 8, crate::world::VERTEX_WHITE);
    merge_into(
        &mut leg,
        &calf,
        Vec3::new(0.0, -thigh - shin * 0.5, 0.0),
        0.0,
    );

    // But: szeroki z przodu, żeby stopa nie zniknęła w nodze.
    //
    // UWAGA na układ współrzędnych — ta siatka jest wyjątkiem.
    //
    // `push_character` rysuje but przez macierz
    //   base * translate(biodro) * obrót * translate(0, -hip_y + ankle_y)
    // czyli ostatnie przesunięcie stawia POCZĄTEK siatki dokładnie na
    // stawie skokowym (`ankle_y`). Wobec tego bryła musi być zbudowana
    // w układzie, którego Y=0 to KOSTKA, a nie podłoga — inaczej
    // `ankle_y` zostaje policzone dwa razy i stopa wisi w powietrzu.
    //
    // Stąd środek = `-ankle_y + SOLE/2`: podeszwa ląduje na 0
    // (na poziomie podłogi), a gora sięga lekko ponad kostkę, żeby
    // podudzie nie kończyło się w powietrzu nad butem.
    const BOOT_HALF_H: f32 = 0.0575;
    let boot = box_mesh(
        Vec3::new(0.0, -r.ankle_y + BOOT_HALF_H, 0.030),
        Vec3::new(0.072, BOOT_HALF_H, 0.115),
        crate::world::VERTEX_WHITE,
    );

    // --- wizjer
    //
    // Płaska soczewka na wysokości oczu. Emisja daje jasny pas na
    // twarzy, który razem z bloomem wygląda jak interfejs kasku.
    let visor = box_mesh(
        Vec3::new(0.0, r.head_y + 0.012, 0.093),
        Vec3::new(0.140, 0.030, 0.020),
        crate::world::VERTEX_WHITE,
    );

    CharacterMeshes {
        head,
        hair,
        hair_bright,
        torso,
        hips,
        arm,
        leg,
        boot,
        buckle,
        visor,
    }
}

/// Podział postaci na części do animacji.
///
/// Animacja to nie tylko ruch całej bryły: obroty stawów dają
/// sylwetce czytelność w trakcie chodu, a statek (chód/bieg/atak/
/// skok) mówi, które stawy się poruszają.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Idle,
    Walk,
    Run,
    Jump,
    Attack,
}

impl Stance {
    /// Amplituda podstawy: jak mocno noga odchyla się od pionu.
    pub fn stride(self) -> f32 {
        match self {
            Stance::Idle => 0.0,
            Stance::Walk => 0.28,
            Stance::Run => 0.62,
            Stance::Jump => 0.18,
            Stance::Attack => 0.10,
        }
    }

    /// Skłon tułowia do przodu. Bieg wymaga przechyłu, inaczej
    /// postać wygląda, jakby stała w miejscu i machała nogami.
    pub fn lean(self) -> f32 {
        match self {
            Stance::Idle => 0.0,
            Stance::Walk => 0.09,
            Stance::Run => 0.26,
            Stance::Jump => -0.06,
            Stance::Attack => 0.16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Najniższy punkt bryły po `offset` — to, o co but styka się
    /// z podłożem.
    fn lowest(mesh: &Mesh, offset: f32) -> f32 {
        mesh.vertices
            .iter()
            .map(|v| v.pos().y + offset)
            .fold(f32::INFINITY, f32::min)
    }

    /// Najwyższy punkt bryły.
    fn highest(mesh: &Mesh, offset: f32) -> f32 {
        mesh.vertices
            .iter()
            .map(|v| v.pos().y + offset)
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// Noga w rzucie na Y musi sięgać DOKŁADNIE do kostki.
    ///
    /// `push_character` umieszcza siatkę nogi w biodrze (`rig.hip_y`)
    /// i oczekuje, że jej dół znajdzie się w `rig.ankle_y` — bo to
    /// tam stawia but. Jeśli noga będzie krótsza, między stopą a
    /// podudziem zostanie szczelina; jeśli dłuższa, noga wjedzie
    /// pod podłogę i postać wygląda na zatopioną po kolana.
    #[test]
    fn leg_reaches_exactly_the_ankle() {
        let m = build_meshes();
        let r = rig();
        // Przesunięcie do biodra jest tym samym, co w `push_character`.
        let bottom = lowest(&m.leg, r.hip_y);
        let top = highest(&m.leg, r.hip_y);
        assert!(
            (bottom - r.ankle_y).abs() < 1e-3,
            "dół nogi jest na {bottom:.4} m, a kostka na {:.4} m (różnica {:.4})",
            r.ankle_y,
            (bottom - r.ankle_y).abs()
        );
        assert!(
            (top - r.hip_y).abs() < 1e-3,
            "góra nogi jest na {top:.4} m, a biodro na {:.4} m",
            r.hip_y
        );
    }

    /// Podudzie nie może wystawać poniżej kostki — to najniższy
    /// punkt nogi i musi być wspólny dla obu stawów.
    #[test]
    fn shin_does_not_dip_below_the_ankle() {
        let m = build_meshes();
        let r = rig();
        // Kolano dzieli nogę na udo i podudzie; kontrolujemy tylko
        // część poniżej kolana, czyli dolną połowę bryły.
        let knee_world = r.hip_y - (r.hip_y - r.knee_y);
        let bottom = m
            .leg
            .vertices
            .iter()
            .map(|v| v.pos().y + r.hip_y)
            .filter(|y| *y < knee_world)
            .fold(f32::INFINITY, f32::min);
        assert!(
            bottom >= r.ankle_y - 1e-3,
            "podudzie schodzi do {bottom:.4} m, czyli {:.4} m pod kostką",
            r.ankle_y - bottom
        );
    }

    /// But stoi na podłożu: jego dół to ~0, a nie głęboko pod nim.
    ///
    /// UWAGA na układ: siatka buta jest zbudowana w układzie, którego
    /// `Y=0` to KOSTKA (nie podłoga) — patrz komentarz przy `boot`
    /// w `build_meshes`. `push_character` dokłada do niej
    /// `translate(0, -hip_y + ankle_y)`, więc w świecie bryła ląduje
    /// na `+ankle_y`. Dlatego do odczytu z siatki dodajemy `ankle_y`,
    /// żeby dostać realne położenie względem podłogi.
    #[test]
    fn boot_sole_sits_on_the_ground() {
        let m = build_meshes();
        let r = rig();
        // Przesunięcie, które `push_character` dokłada do buta.
        let world = r.ankle_y;
        let sole = lowest(&m.boot, world);
        assert!(
            sole > -0.02 && sole < 0.06,
            "podeszwa buta jest na {sole:.4} m zamiast przy 0 (zasięg -0.02..0.06)"
        );
        // Górna krawędź buta musi sięgać co najmniej do kostki, inaczej
        // podudzie kończy się w powietrzu nad stopą.
        assert!(
            highest(&m.boot, world) >= r.ankle_y,
            "but sięga tylko do {:.4} m, a kostka jest na {:.4} m",
            highest(&m.boot, world),
            r.ankle_y
        );
    }
}
