//! Farm Simulator — pole, sadzenie, czekanie, zbiór.
//!
//! # Architektura
//!
//! Gra stoi na sześciu modułach:
//!
//! * [`farm`] — mechanika: siatka działek i pętla sadz → rośnie → zbierz,
//! * [`geometry`] — bryły budowane w kodzie (trawa, roślina, postać),
//! * [`look`] — kierunki w pierwszej osobie (yaw/pitch → wektory),
//! * [`player`] — chód, grawitacja, skok i kolizje,
//! * [`collide`] — świat przeszkód i rozwiązywanie wejść gracza,
//! * [`assets`] — budynki wczytywane z plików `.obj` Quaternius.
//!
//! `main.rs` trzyma tylko warstwę gry: wejście, kamerę, listę
//! rysowania i HUD. Cała logika rozgrywki jest testowana w `farm.rs`,
//! geometria w `geometry.rs`, kierunki w `look.rs`, a fizyka w
//! `player.rs` i `collide.rs` — wszystkie działają bez GPU.
//!
//! Świadomie **bez silnika kolizji** (jak [`junak-rider`]): świat to
//! kilkanaście prostopadłościanów, więc własne AABB jest tańsze o
//! rząd wielkości i łatwiejsze do zrozumienia.
//!
//! # Sterowanie
//!
//! `W/A/S/D` — chodzenie **wzdłuż kierunku patrzenia** (obrót myszą
//! obraca też kierunek chodzenia), `mysz` — obrót kamery,
//! `Spacja` — skok, `E` — sadz lub zbierz, `R` — nowa farma,
//! `F1` — druty kolizji (pomarańczowe bryły świata, błękitny sześcian
//! gracza), `Esc` — wyjście.

mod assets;
mod collide;
mod farm;
mod geometry;
mod look;
mod player;

use std::sync::{Arc, Mutex};

use uran_engine::prelude::*;
use uran_math::{Mat4, Vec3};
use uran_render3d::{DrawCmd, MaterialId, MeshId, Renderer3d, model_matrix};

use crate::farm::{Farm, TILE, tile_at, tile_center};
use crate::geometry::palette;
use crate::look::{clamp_pitch, forward, move_dir, up};

/// Rozmiar świata w metrach. Kwadratowy, bo mgła i kadr cienia
/// zakładają się na jeden wymiar — przy prostokącie trzeba by liczyć
/// dwa różne promienie.
const WORLD: f32 = 90.0;

/// Wysokość oka nad ziemią (typowa dla postaci ~1,7 m).
const EYE: f32 = 1.66;
/// Największa odległość gracza od środka mapy.
const PLAY_LIMIT: f32 = WORLD * 0.5 - 2.0;
/// Czułość myszy w radianach na piksel.
///
/// 0,0035 rad/px ≈ 0,2°/px, czyli pełne 360° wymaga ~1800 px ruchu
/// w poziomie. Poprzednie 0,0022 było zbyt „lepkie" w pierwszej
/// osobie — obrót głowy przy 75° FOV musi być szybki, inaczej
/// przeciwnik (albo krawędź pola) ucieka z kadru, zanim gracz
/// zdąży zareagować. Przy tej wartości krótki ruch nadgarstka
/// daje pełny obrót o 90°, a ramiona nadal kontrolują drobne
/// korekty.
const LOOK_SENS: f32 = 0.0035;

/// Kierunek startowego patrzenia: `yaw = PI` = na `-Z`.
///
/// Gracz startuje **przed** polem (patrz [`Game::new`]), czyli po
/// stronie `+Z` osi, a samo pole leży w środku świata, czyli w `0`.
/// Żeby widzieć pole, trzeba patrzeć na `-Z`. `yaw = 0` patrzyłoby
/// w przeciwną stronę, na pustą trawę — stąd stała zamiast `0.0`.
const START_YAW: f32 = std::f32::consts::PI;
/// Startowy pitch: lekko w dół, na pole przed sobą.
///
/// -0,22 rad (≈12,6°) to tyle, żeby ziemia zajmowała dolne ~2/3
/// kadru, a horyzont i budynki górne 1/3. Zerowy pitch kazałby
/// patrzeć w niebo i pole ledwo by było widać.
const START_PITCH: f32 = -0.22;
/// Odległość startu od **płotu**, nie od pola.
///
/// Płot stoi na `fd/2 + 0.5` i ma 1,1 m wysokości. Oko gracza jest na
/// wysokości 1,66 m, więc stojąc tuż za płotem, gracz patrzyłby
/// przez jego szczebelnicę — dolna połowa kadru to były drewno.
/// 7 m odsuwa gracza na tyle, żeby płot znalazł się u dołu ekranu
/// jako ramka, a nie jako przeszkoda.
const START_OFFSET: f32 = 7.0;

/// Najwyższe położenie stóp gracza (m) — sufit dla skoku.
///
/// Zabezpieczenie przed sytuacją, w której kamera ucieka w górę: gdyby
/// kolidator kiedyś wypchnął gracza w osi Y (dziś tego nie robi —
/// [`collide::World`] rozwiązuje tylko X i Z), pozycja stóp i tak nie
/// przekroczyłaby tej wartości. Skok osiąga ~1,4 m, więc zapas jest
/// duży, a błąd w kodzie nie objawiłby się jako „latanie po mapie".
const MAX_FOOT_Y: f32 = 6.0;

/// Budynki na mapie: plik, pozycja środka, obrót i obrys w metrach.
///
/// Jedno źródło prawdy dla rysowania ([`build_props`]) i dla kolizji
/// ([`build_collision_world`]). Osobne listy zaprosiłyby do błędu:
/// przesunięcie stodoła w rysowaniu dałoby graczowi wejście w jego
/// środek.
///
/// Obrys (`width`/`depth`) to **naturalne** wymiary modelu, a nie AABB
/// po obrocie — to drugie liczy już [`build_collision_world`]. Rozmiar
/// podajemy z plików `.obj` (Quaternius, oś Y): stodoło 7,7 × 9,1 m,
/// silos 3,7 × 3,5 m, młyn 4,4 × 4,4 m, studnia 1,4 × 1,4 m.
const PROP_PLACES: &[(&str, Vec3, f32, f32, f32, f32)] = &[
    (
        "BigBarn",
        Vec3::new(-14.0, 0.0, -11.0),
        0.35,
        7.7,
        9.1,
        7.89,
    ),
    ("Silo", Vec3::new(-6.0, 0.0, -14.5), 0.0, 3.7, 3.5, 9.07),
    (
        "Windmill",
        Vec3::new(14.0, 0.0, -11.0),
        -0.45,
        4.4,
        4.4,
        11.20,
    ),
    ("Well", Vec3::new(4.5, 0.0, 8.0), 0.8, 1.4, 1.4, 2.15),
];

/// Jedna część wczytanego budynku: lista (siatka, materiał).
struct Prop {
    parts: Vec<(MeshId, MaterialId)>,
    pos: Vec3,
    yaw: f32,
}

/// Siatki zarejestrowane raz przy starcie.
struct Meshes {
    ground: MeshId,
    soil: MeshId,
    /// Roślina w trzech fazach: sadzonka, w połowie, dojrzała.
    plant: [MeshId; 3],
    /// Budynki i płot wokół farmy.
    buildings: Vec<Prop>,
    /// Kubit 1×1×1 m wokół środka — drunek kolidatora. Rysujemy go
    /// skalą per bryła, więc **jedna** siatka obsługuje wszystkie
    /// kolidatory, niezależnie od ich rozmiaru.
    collider: MeshId,
    /// Sześcian gracza — pokazuje, gdzie kończy się ciało przy
    /// sprawdzaniu kolizji.
    player_box: MeshId,
}

/// Stan gry dzielony między systemami.
struct Game {
    scene: Arc<Mutex<Renderer3d>>,
    meshes: Meshes,
    farm: Farm,
    /// Świat przeszkód: budynki, płot i granica mapy.
    world: collide::World,
    /// Chód, grawitacja i skok.
    player: player::Player,
    /// Parametry chodu i skoku (prędkość, grawitacja, wysokość skoku).
    player_cfg: player::PlayerConfig,
    /// Pozycja gracza na płaszczyźnie.
    player_pos: Vec3,
    /// Kierunek patrzenia: obrót wokół Y (poziom) i X (pion).
    ///
    /// Trzymamy kąty, a nie wektor, bo obrót myszą to dodawanie do
    /// kąta, a nie mnożenie wektora. `pitch` jest zawsze przycięty
    /// przez [`clamp_pitch`], więc nigdy nie wychodzi poza pion.
    yaw: f32,
    pitch: f32,
    /// Skąd patrzymy: wektor obliczany z `yaw`/`pitch` razem z
    /// pozycją oka. Osobno trzymany, bo `build()` (rysowanie) ma
    /// tylko odczyt, a `update()` go oblicza.
    eye: Vec3,
    /// Kierunek patrzenia, także wektor — do rysowania celownika
    /// i do obliczania, gdzie gracz patrzy przy `E`.
    view_dir: Vec3,
    /// Faza kołysania kroku (radiany). Rośnie z **przebytą
    /// drogą**, nie z czasem, więc stojący gracz nie kołysze się,
    /// a chodzący zawsze ma ten sam rytm niezależnie od `dt`.
    walking: f32,
    /// Wygładzona prędkość chodu 0..1 — do rozmycia kołysania przy
    /// ruszaniu i zatrzymywaniu. Bez tego oko szarpnęłoby przy każdym
    /// naciśnięciu i puszczeniu klawisza.
    walk_blend: f32,
    /// Czas świata (mgła i animacje).
    time: f32,
    /// Czy wszystkie działki są dojrzałe.
    won: bool,
    /// Czy rysować druty kolizji (`F1`).
    show_colliders: bool,
    font: Option<Font>,
}

impl Game {
    /// Buduje świat i renderer 3D na gotowym urządzeniu.
    fn new(gpu: &uran_render::GpuContext, format: wgpu::TextureFormat, samples: u32) -> Self {
        let mut scene = Renderer3d::new(gpu, format, samples);
        // Niebo: jasny błękit. Kolor podajemy w sRGB, bo shader sceny
        // miesza z nim mgłę i musi zgadzać się z gradientem nieba.
        scene.set_clear_color([0.58, 0.74, 0.90, 1.0]);
        {
            let l = scene.lighting_mut();
            // Słońce ~48° nad horyzontem i wyraźnie z boku. Niski kąt
            // daje długie cienie, które pokazują, gdzie jest pole
            // uprawne, a gdzie zwykła trawa.
            l.light_dir = Vec3::new(0.42, 0.66, 0.62).normalize();
            // Ciepłe, lekko złotawe — popołudniowe słońce nad polem.
            l.light_color = [1.0, 0.95, 0.84];
            l.ambient = [0.36, 0.45, 0.60];
        }
        {
            let cam = scene.camera_mut();
            // 75°: pierwsza osoba potrzebuje szerszego pola widzenia niż
            // kamera trzecioosobowa. Przy 60° budynki przy krawędziach
            // kadru były mocno zniekształcone, a horyzont ucinał się
            // zbyt blisko — przy skręcie myszą było „wąsko".
            cam.fov_y = 75.0_f32.to_radians();
            cam.near = 0.1;
            // Dalej płaszczyzna: przy oknie 90 m i budynku 11 m wysokości
            // 200 m w zupełności wystarcza, a mniejsza wartość pozwala
            // precyzyjniej dzielić głębokość w buforze cieni.
            cam.far = 200.0;
        }

        // --- teren i pole ---
        let ground = geometry::ground(WORLD, 6.0);
        let ground = scene.add_mesh(gpu, &ground, "trawa");

        // Gleba: jedna płyta pod całym polem. Działki różnią się tym,
        // co na nich rośnie, a nie kolorem gruntu — inaczej zamiast
        // uprawy widać by szachownicę.
        let fw = farm::FIELD_W as f32 * TILE;
        let fd = farm::FIELD_D as f32 * TILE;
        // Podnosimy glebę o 2 cm nad trawę: inaczej z poziomu kamery
        // obie powierzchnie zlewają się w jedną płaszczyznę.
        let soil = scene.add_mesh(
            gpu,
            &geometry::quad_y(0.0, 0.0, fw * 0.5, 0.02, palette::SOIL),
            "gleba",
        );

        // --- roślina w trzech fazach wzrostu ---
        let plant = [
            scene.add_mesh(gpu, &geometry::plant(0.12), "roslina 0"),
            scene.add_mesh(gpu, &geometry::plant(0.55), "roslina 1"),
            scene.add_mesh(gpu, &geometry::plant(1.0), "roslina 2"),
        ];

        // --- druty kolizji (F1) ---
        //
        // **Kubit jednostkowy** wokół środka: skalą do `half * 2`
        // dajemy dowolny rozmiar, więc jedna siatka obsługuje wszystkie
        // kolidatory. Grubość drutu jest w metrach i nie skaluje się —
        // przy skali kolidatora `half` w metrach, podział trójek jest
        // mniej mylący niż ułamek proporcji.
        //
        // Kolidator mnożymy przez `2 * half` w [`build_draw_list`],
        // a stąd wewnątrz siatki zostaje grubość 6 cm na krawędzi kubita
        // jednostkowego — po skalowaniu daje 3 cm na ścianie płotu
        // i kilkanaście centymetrów na stodole.
        let collider = scene.add_mesh(
            gpu,
            &geometry::collider_box(
                Vec3::ZERO,
                Vec3::new(0.5, 0.5, 0.5),
                geometry::COLLIDER,
                0.03,
            ),
            "kolidator",
        );
        let player_box = scene.add_mesh(
            gpu,
            &geometry::collider_box(
                Vec3::ZERO,
                Vec3::new(0.5, 0.5, 0.5),
                geometry::PLAYER_BOX,
                0.06,
            ),
            "gracz-box",
        );

        // Siatki postaci nie ma: w pierwszej osobie oko gracza siedzi
        // wewnątrz jego głowy, więc narysowana postać zasłoniłaby cały
        // ekran. Zamiast niej HUD rysuje celownik.
        let _ = &geometry::player;

        // --- płot i budynki wokół farmy ---
        let buildings = build_props(&mut scene, gpu);

        println!(
            "🌾  farma: {}×{} działek, {}×{:.0} m, {}/{} budynków",
            farm::FIELD_W,
            farm::FIELD_D,
            fw,
            fd,
            assets::available_buildings(),
            assets::BUILDINGS.len()
        );

        Self {
            scene: Arc::new(Mutex::new(scene)),
            meshes: Meshes {
                ground,
                soil,
                plant,
                buildings,
                collider,
                player_box,
            },
            farm: Farm::new(),
            // Startujemy przed polem i patrzymy **na** pole: pozycja
            // jest po stronie `+Z`, więc kierunek patrzenia to `-Z`
            // (`START_YAW`). `W` od razu prowadzi na działki.
            player: {
                let mut p = player::Player::default();
                p.teleport(0.0, farm::FIELD_D as f32 * TILE * 0.5 + START_OFFSET);
                p
            },
            world: build_collision_world(),
            player_cfg: player::PlayerConfig::default(),
            player_pos: Vec3::new(0.0, 0.0, fd * 0.5 + START_OFFSET),
            yaw: START_YAW,
            pitch: START_PITCH,
            eye: Vec3::new(0.0, EYE, fd * 0.5 + START_OFFSET),
            view_dir: Vec3::new(0.0, 0.0, -1.0),
            walking: 0.0,
            walk_blend: 0.0,
            time: 0.0,
            won: false,
            show_colliders: false,
            font: None,
        }
    }
}

/// Rejestruje wczytany model: część na część, materiał na materiał.
///
/// Materiały deduplikujemy po nazwie: ten sam kolor z `.mtl` występuje
/// w kilku częściach modelu, a każda kopia to osobny bind group
/// i osobne tekstury na karcie.
fn register_model(
    scene: &mut Renderer3d,
    gpu: &uran_render::GpuContext,
    model: uran_render3d::ImportedModel,
) -> Vec<(MeshId, MaterialId)> {
    let mut seen: Vec<(String, MaterialId)> = Vec::new();
    let mut out = Vec::with_capacity(model.parts.len());
    for part in &model.parts {
        let mat = match seen.iter().find(|(n, _)| *n == part.material.name) {
            Some((_, id)) => *id,
            None => {
                let id = scene.add_material(gpu, &part.material);
                seen.push((part.material.name.clone(), id));
                id
            }
        };
        let id = scene.add_mesh(gpu, &part.mesh, &part.name);
        out.push((id, mat));
    }
    out
}

/// Rozstawienie płotu i budynków wokół pola.
///
/// Pozycje dobrane tak, żeby z dowolnego miejsca pola widać było
/// co najmniej jeden budynek — puste niebo wygląda jak brakujący świat,
/// a nie łąka otoczona lasem.
fn build_props(scene: &mut Renderer3d, gpu: &uran_render::GpuContext) -> Vec<Prop> {
    let mut out: Vec<Prop> = Vec::new();
    let fw = farm::FIELD_W as f32 * TILE;
    let fd = farm::FIELD_D as f32 * TILE;

    // --- płot wokół pola ---
    //
    // Segment `Fence.obj` ma 5,89 m długości przy naturalnej skali
    // (1,10 m wysokości), więc rozstawiamy go co ~5,5 m z lekkim
    // zakładką. Wcześniejszy krok 2,2 m kładł segmenty JEDEN NA
    // DRUGIM — na zrzucie widać by „kupę belek" zamiast płotu.
    if let Some(model) = assets::load_building("Fence", assets::FENCE_HEIGHT) {
        let fence = register_model(scene, gpu, model);
        // 5,5 m: tyle ma segment pomniejszony o zakładkę, żeby słupki
        // stykały się wizualnie bez szczeliny.
        let step = 5.5f32;
        let half_w = fw * 0.5 + 0.5;
        let half_d = fd * 0.5 + 0.5;
        // `ceil` w górę + `min 2` — pole musi mieć choćby dwa segmenty
        // na bok, inaczej płot wygląda jak jedna belka.
        let nx = (fw / step).ceil().max(2.0) as i32;
        let nz = (fd / step).ceil().max(2.0) as i32;
        // `false` = wiersz wzdłuż X, `true` = wzdłuż Z.
        for (along_z, z_edge, x_edge, yaw, n) in [
            (false, -half_d, 0.0f32, 0.0f32, nx),
            (false, half_d, 0.0, std::f32::consts::PI, nx),
            (true, 0.0, -half_w, std::f32::consts::FRAC_PI_2, nz),
            (true, 0.0, half_w, -std::f32::consts::FRAC_PI_2, nz),
        ] {
            for k in 0..n {
                // `-0.5 .. +0.5` to środek pierwszego i ostatniego
                // segmentu, dzięki czemu płot jest symetryczny.
                let t = (k as f32 + 0.5) / n as f32 - 0.5;
                let pos = if along_z {
                    Vec3::new(x_edge, 0.0, t * n as f32 * step)
                } else {
                    Vec3::new(t * n as f32 * step, 0.0, z_edge)
                };
                out.push(Prop {
                    parts: fence.clone(),
                    pos,
                    yaw,
                });
            }
        }
    }

    // --- budynki ---
    //
    // Każdy ma własny obrót: stodoło prostopadle do młyna czyta się
    // jako wieś, a wszystko równolegle — jako parking.
    // Pole ma 12×12 m, więc budynki stawiamy od 9 m od jego środka
    // w bok — inaczej stodoło (7,7 m szerokości) zasłaniałoby uprawę.
    for (file, pos, yaw, _w, _d, _h) in PROP_PLACES {
        let Some(spec) = assets::BUILDINGS.iter().find(|b| b.file == *file) else {
            continue;
        };
        let Some(model) = assets::load_building(file, spec.height) else {
            continue;
        };
        out.push(Prop {
            parts: register_model(scene, gpu, model),
            pos: *pos,
            yaw: *yaw,
        });
    }

    out
}

/// Buduje świat kolizji z **tych samych** danych co [`build_props`].
///
/// Pozycje budynków żyją w jednej stałej [`PROP_PLACES`], z której
/// korzystają obie funkcje. Duplikowanie ich w dwóch miejscach to
/// zaproszenie do błędu: wystarczy przesunąć stodoło w rysowaniu
/// i gracz przechodzi przez jego środek.
fn build_collision_world() -> collide::World {
    let fw = farm::FIELD_W as f32 * TILE;
    let fd = farm::FIELD_D as f32 * TILE;
    let mut w = collide::World::new(PLAY_LIMIT);

    // --- płot wokół pola ---
    //
    // Cztery **ciągłe** ściany, nie po jednym segmencie: gracz nie
    // powinien wciskać się w szczelinę między słupkami. Ściana ma
    // 0,3 m grubości i pełną wysokość płotu, więc przeskoczyć go
    // można, ale przejść obok — nie da się wejść na pole inaczej
    // niż przez bramę.
    let half_w = fw * 0.5 + 0.5;
    let half_d = fd * 0.5 + 0.5;
    let t = 0.3;
    for (x, z, width, depth) in [
        // północna i południowa
        (0.0f32, -half_d, fw + 1.0, t),
        (0.0, half_d, fw + 1.0, t),
        // wschodnia i zachodnia
        (-half_w, 0.0, t, fd + 1.0),
        (half_w, 0.0, t, fd + 1.0),
    ] {
        w.add(collide::Solid {
            pos: Vec3::new(x, 0.0, z),
            width,
            depth,
            height: assets::FENCE_HEIGHT,
        });
    }

    // --- budynki ---
    //
    // Rozmiary bierzemy z naturalnych proporcji modeli, a nie z AABB
    // modelu — budynki stoją na trawie, więc gracz nie wchodzi w ich
    // wnętrze przez dach.
    for (_, pos, yaw, width, depth, height) in PROP_PLACES {
        // Obrót o `yaw` zmienia obrys w XZ, więc bierzemy obrys
        // prostokąta **po obrocie**. Inaczej gracz wchodziłby w róg
        // obróconej bryły albo blokował się o powietrze przy jej
        // krawędzi.
        // `abs()` na wyniku metody, a nie na niej samej: `x.sin().abs()`
        // daje `f32`, a `x.abs().sin()` już nie.
        let (s, c) = (yaw.sin().abs(), yaw.cos().abs());
        w.add(collide::Solid {
            pos: *pos,
            width: width * c + depth * s,
            depth: width * s + depth * c,
            height: *height,
        });
    }

    w
}

/// Macierz postaci.
///
/// Postać rysowana jest tylko wtedy, gdy kamera pierwszoosobowa
/// patrzy w dół wystarczająco (patrz [`build_draw_list`]) — wtedy
/// obrót postaci ma znaczenie, bo widzimy jej ramiona i tors. Przy
/// patrzeniu prosto przed siebie sylwetki nie widać, więc `yaw`
/// jest skądś „z definicji" zerowy i nie próbujemy zgadywać, którą
/// stronę ma bark.
fn player_matrix(p: Vec3) -> Mat4 {
    model_matrix(p, 0.0, 0.0, 0.0, Vec3::ONE)
}

/// Kamera pierwszoosobowa: oko gracza, cel w kierunku patrzenia.
///
/// Pozycja oka to `player_pos` podniesiony o [`EYE`] — wysokość
/// oczu człowieka, a nie „wysokość głowy modelu". Kierunek celu to
/// `eye + view_dir`, czyli wektor jednostkowy [`look::forward`].
///
/// Wcześniej kamera była trzecioosobowa (`+CAM_BACK` w Z, `+CAM_UP`
/// w Y) i patrzyła na gracza. Przy pierwszej osobie takie ustawienie
/// oznaczałoby, że gracz patrzy **odwrotnie** niż idzie.
fn update_camera(game: &mut Game) {
    if let Ok(mut scene) = game.scene.lock() {
        let cam = scene.camera_mut();
        cam.position = game.eye;
        // Cel wyliczamy z kierunku, nie z kątów — dzięki temu kamera
        // zawsze jest spójna z tym, co narysował `look::forward`, nawet
        // jeśli kąty zostaną kiedyś przycięte inaczej.
        cam.target = game.eye + game.view_dir;
    }
}

/// Drobne przesunięcie rośliny w obrębie działki, żeby nie stały
/// w idealnym szyku.
///
/// Używamy liczb „współczynników złotego ciągu" zamiast `rand`, bo:
/// * ten sam `(ix, iz)` daje zawsze to samo przesunięcie — rośliny
///   nie skaczą między klatkami,
/// * nie potrzebujemy stanu generatora w strukturze gry.
fn jitter(ix: usize, iz: usize) -> (f32, f32) {
    let a = 0.55 * (0.618_034 * (ix as f32 * 7.0 + iz as f32 * 3.0).sin());
    let b = 0.55 * (0.618_034 * (ix as f32 * 3.0 + iz as f32 * 7.0).cos());
    (a, b)
}

/// Składa listę obiektów 3D na bieżącą klatkę.
///
/// Kolejność ma znaczenie tylko dla czytelności — głębią zajmuje się
/// depth buffer, a potok nie ma mieszania alfa.
fn build_draw_list(game: &mut Game) {
    let mut cmds: Vec<DrawCmd> = Vec::with_capacity(160);
    let one = model_matrix(Vec3::ZERO, 0.0, 0.0, 0.0, Vec3::ONE);
    let m = &game.meshes;

    // Trawa i gleba leżą nieruchomo.
    cmds.push(DrawCmd::new(m.ground, one));
    cmds.push(DrawCmd::new(m.soil, one));

    // Płot i budynki.
    for prop in &m.buildings {
        let mat = model_matrix(prop.pos, prop.yaw, 0.0, 0.0, Vec3::ONE);
        for (id, mat_id) in &prop.parts {
            cmds.push(DrawCmd::new(*id, mat).with_material(*mat_id));
        }
    }

    // Rośliny: dla każdej zasadzonej działki dobieramy siatkę wg
    // wzrostu. Trzy siatki zamiast skaliowania jednej, bo skalowanie
    // zniekształcałoby proporcje łodygi.
    for iz in 0..farm::FIELD_D {
        for ix in 0..farm::FIELD_W {
            let tile = game.farm.get(ix, iz);
            if tile == farm::Tile::Empty {
                continue;
            }
            let g = tile.growth();
            let stage = if tile.is_ripe() {
                2
            } else if g < 0.5 {
                0
            } else {
                1
            };
            let (jx, jz) = jitter(ix, iz);
            let pos = tile_center(ix, iz) + Vec3::new(jx, 0.0, jz);
            cmds.push(DrawCmd::new(m.plant[stage], player_matrix(pos)));
        }
    }

    // Druty kolizji (`F1`) — na końcu, żeby nie mieszały się z
    // roślinami przy sortowaniu i były zawsze na wierzchu.
    push_collider_debug(game, &mut cmds);

    // Gracz.
    if let Ok(mut scene) = game.scene.lock() {
        scene.set_commands(cmds);
    }
}

/// Druty kolizji: pomarańczowe bryły świata + błękitny sześcian gracza.
///
/// Rysujemy **krawędzie** (patrz [`geometry::collider_box`]), więc nic
/// nie zasłania i widać, gdzie naprawdę kończy się przeszkoda.
/// Sześcian gracza pokazuje promień kolizji, który jest niewidoczny —
/// bez niego gracz wydaje się wchodzić w ścianę „do połowy".
///
/// Debug bez GPU: `update` przełącza tryb klawiszem `F1`, a stan
/// pokazuje `draw_hud`.
fn push_collider_debug(game: &Game, cmds: &mut Vec<DrawCmd>) {
    if !game.show_colliders {
        return;
    }
    let m = &game.meshes;
    for b in game.world.boxes() {
        // Siatka to kubit jednostkowy, więc skalujemy do pełnej bryły:
        // `2 * half` daje szerokość 2 m przy `half = 1`.
        let scale = Mat4::from_scale(b.half * 2.0);
        cmds.push(DrawCmd::new(
            m.collider,
            Mat4::from_translation(b.center) * scale,
        ));
    }
    // Sześcian gracza: dolna krawędź na wysokości stóp, więc przesuwamy
    // o pół wysokości. Środek `pos` to stopy, a nie środek ciała.
    let feet = game.player.pos;
    let half = Vec3::new(
        collide::PLAYER_RADIUS,
        collide::PLAYER_HEIGHT * 0.5,
        collide::PLAYER_RADIUS,
    );
    cmds.push(DrawCmd::new(
        m.player_box,
        Mat4::from_translation(feet + Vec3::new(0.0, collide::PLAYER_HEIGHT * 0.5, 0.0))
            * Mat4::from_scale(half * 2.0),
    ));
}

/// Krok symulacji + wejście.
fn update(ctx: &mut Ctx, game: &mut Game) {
    let dt = ctx.dt();
    game.time += dt;
    if let Ok(mut scene) = game.scene.lock() {
        scene.advance(dt);
    }

    if ctx.input.just_pressed(Key::Escape) {
        std::process::exit(0);
    }
    // `F1` przełącza druty kolizji. Bez podpowiedzi w HUD-u tryb
    // wyglądałby jak błąd renderowania, więc stan pokazujemy w
    // lewym dolnym rogu (patrz `draw_hud`).
    if ctx.input.just_pressed(Key::F1) {
        game.show_colliders = !game.show_colliders;
    }

    if ctx.input.just_pressed(Key::KeyR) {
        game.farm = Farm::new();
        game.won = false;
        // Reset przywraca dokładnie stan startowy — także pozycję
        // i kierunek patrzenia. Bez tego gracz stałby tyłem do pola
        // albo na środku uprawy.
        game.player
            .teleport(0.0, farm::FIELD_D as f32 * TILE * 0.5 + START_OFFSET);
        game.player_pos = game.player.pos;
        game.yaw = START_YAW;
        game.pitch = START_PITCH;
        game.walking = 0.0;
        game.walk_blend = 0.0;
    }

    // --- obrót kamery myszą ---
    //
    // `mouse_delta()` to ruch względny (nie pozycja kursora), więc
    // obrót nie skacze, gdy kursor znalazł się w rogu okna. Silnik
    // limituje deltę do 200 px, co chroni przed skokiem po powrocie
    // z alt-tab.
    //
    // Znak `-=` dla `yaw`: przesunięcie myszy w prawo ma obrócić kamerę
    // w prawo, czyli zmniejszyć kąt, bo dodatni `yaw` skręca w lewo
    // (konwencja matematyczna).
    let m = ctx.input.mouse_delta();
    if m.x != 0.0 || m.y != 0.0 {
        game.yaw -= m.x * LOOK_SENS;
        game.pitch = clamp_pitch(game.pitch - m.y * LOOK_SENS);
    }
    // `yaw` rośnie w nieskończoność — przy 100 obrotach myszy
    // wychodzi już tysiąc radianów i `sin_cos` traci precyzję.
    // Składamy do `-PI..PI`.
    if game.yaw > std::f32::consts::PI || game.yaw < -std::f32::consts::PI {
        game.yaw = game.yaw.rem_euclid(std::f32::consts::TAU);
        if game.yaw > std::f32::consts::PI {
            game.yaw -= std::f32::consts::TAU;
        }
    }

    // --- ruch gracza ---
    //
    // Dwie składowe wejścia: `W/S` na osi „do przodu / do tyłu" względem
    // kierunku patrzenia i `A/D` na „w lewo / w prawo". Oś pionowa jest
    // zawsze `0` — gracz nie lata i nie chodzi po ścianach.
    //
    // Wszystko liczy [`look::move_dir`], który **normalizuje** wynik.
    // Bez tego `W` + `D` dawałoby √2 × WALK_SPEED, czyli chód po skosie
    // byłby szybszy niż po prostej.
    let fwd_in = ctx.input.axis(Key::KeyS, Key::KeyW);
    let strafe_in = ctx.input.axis(Key::KeyA, Key::KeyD);
    // `move_dir` bierze `yaw`, więc obrót myszą obraca też kierunek
    // chodzenia — to właśnie ta własność, o którą prosiłeś.
    let dir = move_dir(game.yaw, fwd_in, strafe_in);
    // Ruch: `Player::step` liczy chód w kierunku `dir`, grawitację
    // i skok, a `collide::World` rozwiązuje wejścia w budynki i płot.
    //
    // Krok wykonujemy **zawsze**, także gdy `dir` jest zerowe:
    // inaczej gracz zastyłby w powietrzu po wypchnięciu ze skoku
    // i nigdy by nie wylądował.
    // Skakamy **wciśnięciem** Spacji (`just_pressed`), nie jej
    // trzymaniem — inaczej gracz wskakiwałby w miejscu przy każdym
    // lądowaniu, bo `pressed` jest prawdziwe przez cały czas wciśnięcia.
    let jump = ctx.input.just_pressed(Key::Space);
    game.player.step(
        dt,
        dir,
        jump,
        &game.world,
        collide::PLAYER_RADIUS,
        collide::PLAYER_HEIGHT,
        &game.player_cfg,
    );

    // Ograniczenie do kwadratu świata: bez niego gracz wychodzi
    // poza teren i kamera pokazuje pustą mgłę. `Player::step` pilnuje
    // przeszkód, więc granicę mapy trzymamy osobno.
    let limit = game.world.limit;
    game.player.pos.x = game.player.pos.x.clamp(-limit, limit);
    game.player.pos.z = game.player.pos.z.clamp(-limit, limit);

    if dir.length_squared() > 1e-6 {
        // Faza rośnie o **przebytą** drogę (ok. 2 kroki na metr), a nie
        // o czas — dzięki temu rytm kołysania nie zależy od `dt`
        // i przy 30 FPS wygląda tak samo jak przy 144.
        let step = dir * (game.player_cfg.walk_speed * dt);
        game.walking += step.length() * 6.0;
    }
    // `walk_blend` wygładza start i stop. `1 - exp(-k*dt)` to
    // wygładzanie niezależne od częstotliwości klatek: przy małym `dt`
    // zbliża się do `k*dt`, przy dużym nie "przeskakuje" do 1.
    // Stać w miejscu ma wygaszać szybciej niż ruszać.
    let target = f32::from(dir.length_squared() > 1e-6);
    let k = if target > game.walk_blend { 11.0 } else { 16.0 };
    game.walk_blend += (target - game.walk_blend) * (1.0 - (-k * dt).exp());

    // Oko trzymamy w jednym miejscu, żeby rysowanie nie musiało
    // zgadywać, co widzi gracz. `eye` unosi się do wysokości oczu,
    // `view_dir` to pełny kierunek z pitchem.
    game.view_dir = forward(game.yaw, game.pitch);

    // Pozycja stóp jest przycięta do `0..=MAX_FOOT_Y`.
    //
    // Dolna granica to podłoga: bez niej oko schodziłoby pod ziemię
    // przy pierwszym kroku w dół. Górna to **zabezpieczenie**, nie
    // mechanika skoku — `Player::step` sam pilnuje grawitacji, a skok
    // osiąga ~1,4 m przy suficie 6 m. Bez tej górnej granicy każdy
    // błąd w fizyce objawiałby się jako „kamera ucieka w górę" zamiast
    // zrzutu błędu, a taki objaw jest bardzo trudny do zlokalizowania.
    let feet = game.player.pos;
    let feet = Vec3::new(feet.x, feet.y.clamp(0.0, MAX_FOOT_Y), feet.z);
    game.player.pos.y = feet.y;
    game.player_pos = feet;

    // Lekkie kołysanie przy chodzeniu. Przesuwamy oko wzdłuż osi
    // „w górę ekranu" (`look::up`), a nie wzdłuż globalnego Y — dzięki
    // temu głowa chodzi w górę i w dół także wtedy, gdy gracz patrzy
    // pod nogi, i kołysanie nie zmienia kierunku patrzenia.
    let bob = game.walking.sin() * 0.045 * game.walk_blend;
    game.eye = feet + Vec3::new(0.0, EYE, 0.0) + up(game.yaw, game.pitch) * bob;

    // --- interakcja z polem ---
    //
    // W pierwszej osobie gracz nie stoi na działce, tylko obok niej,
    // więc `E` musi działać na tę działkę, **którą wskazuje**, nie
    // tę, na której stoi. Bierzemy więc punkt przed graczem.
    if ctx.input.just_pressed(Key::KeyE) {
        let target = game.eye + game.view_dir * farm::REACH;
        match tile_at(target) {
            Some((ix, iz)) => {
                game.farm.interact(ix, iz);
            }
            None => {
                game.farm.message = "Nie ma tu pola".into();
                game.farm.message_time = 1.2;
            }
        }
    }

    // Rośliny dojrzewają niezależnie od tego, czy gracz patrzy.
    game.farm.step(dt);
    if !game.won && game.farm.is_won() {
        game.won = true;
    }

    update_camera(game);
    build_draw_list(game);
}

/// HUD: stan pola, liczniki i podpowiedź sterowania.
///
/// Rysujemy w przestrzeni EKRANU (renderer 2D), tak jak w `uran-tanks`
/// i `junak-rider`, i z tej samego powodu: 2D nie przechodzi przez
/// post-processing 3D, więc tekst nie zostaje zniekształcony przez ACES
/// ani rozmyty przez bloom.
fn draw_hud(ctx: &mut Ctx, game: &Game) {
    let size = ctx.window.logical_size();
    let (empty, growing, ready) = game.farm.counts();

    // --- pasek stanu pola (u góry ekranu) ---
    ctx.gfx
        .screen_space()
        .layer(100)
        .color(Color::from_hex(0x1B2430))
        .draw_rect(Rect::new(
            Vec2::new(size.x * 0.5 - 190.0, 14.0),
            Vec2::new(size.x * 0.5 + 190.0, 44.0),
        ));
    // Proporcja paska to `ready / wszystkie`: to stan, który gracz
    // realnie zmienia — sadzenie samo w sobie nie kończy pracy.
    let total: f32 = (empty + growing + ready).max(1) as f32;
    let t = ready as f32 / total;
    ctx.gfx
        .color(Color::from_hex(0xE8C547))
        .draw_rect(Rect::new(
            Vec2::new(size.x * 0.5 - 188.0, 38.0),
            Vec2::new(size.x * 0.5 - 188.0 + 376.0 * t, 42.0),
        ));

    // --- celownik ---
    //
    // Kursor systemowy jest ukryty (`cursor_visible(false)` na oknie),
    // bo w pierwszej osobie mysz steruje kamerą, a nie wskazuje. Bez
    // własnego celownika gracz traci orientację, gdzie patrzy.
    //
    // Cztery krótkie „ramiona" zamiast pełnego krzyża: nie zasłaniają
    // tego, co jest pod nimi, a mimo to jednoznacznie wskazują środek
    // ekranu. Każde ramię rysujemy dwa razy — ciemnym prostokątem
    // o 1 px większym pod spodem, potem białym — żeby celownik
    // był widoczny i na jasnym trawniku, i na ciemnym budynku.
    {
        let c = Vec2::new(size.x * 0.5, size.y * 0.5);
        const ARM: f32 = 7.0;
        const GAP: f32 = 3.0;
        let arms = [
            // poziome: lewe i prawe
            (Vec2::new(c.x - GAP - ARM, c.y), Vec2::new(c.x - GAP, c.y)),
            (Vec2::new(c.x + GAP, c.y), Vec2::new(c.x + GAP + ARM, c.y)),
            // pionowe: górne i dolne
            (Vec2::new(c.x, c.y - GAP - ARM), Vec2::new(c.x, c.y - GAP)),
            (Vec2::new(c.x, c.y + GAP), Vec2::new(c.x, c.y + GAP + ARM)),
        ];
        for (a, b) in arms {
            let outline = Rect::new(
                Vec2::new(a.x - 1.0, a.y - 1.0),
                Vec2::new(b.x + 1.0, b.y + 1.0),
            );
            ctx.gfx.color(Color::from_hex(0x1B2430)).draw_rect(outline);
            ctx.gfx
                .color(Color::from_hex(0xF4F7FB))
                .draw_rect(Rect::new(a, b));
        }
    }

    // Wskaźnik działki przed graczem: pokazuje, co stanie się po `E`.
    // Wskazujemy tam, gdzie patrzymy (`eye + view_dir * REACH`), bo w
    // pierwszej osobie gracz nie stoi na polu, tylko obok niego.
    if let Some((ix, iz)) = tile_at(game.eye + game.view_dir * farm::REACH) {
        let (hint, c) = match game.farm.get(ix, iz) {
            farm::Tile::Empty => ("[E] Zasadz", Color::from_hex(0x9BE07A)),
            farm::Tile::Ready => ("[E] Zbierz", Color::from_hex(0xF2C14E)),
            farm::Tile::Growing { .. } => ("Rosnie...", Color::from_hex(0xB9C4D4)),
        };
        if let Some(font) = game.font {
            ctx.gfx.color(c).draw_text(
                font,
                hint,
                Vec2::new(size.x * 0.5, size.y * 0.5 - 90.0),
                26.0,
                TextAlign::Center,
            );
        }
    }

    // --- komunikat akcji ---
    if game.farm.message_time > 0.0 {
        if let Some(font) = game.font {
            ctx.gfx.color(Color::from_hex(0xF4F7FB)).draw_text(
                font,
                &game.farm.message,
                Vec2::new(size.x * 0.5, size.y * 0.5 - 130.0),
                22.0,
                TextAlign::Center,
            );
        }
    }

    let Some(font) = game.font else { return };

    // --- liczniki (lewy górny róg) ---
    //
    // UWAGA: tylko znaki ASCII. Polskie diakrytyczne wywołują błąd
    // atlasu glifów w `uran-render` przy przepełnieniu (patrz
    // `draw_hud` w uran-tanks), więc HUD zostaje czytelny bez nich.
    ctx.gfx.color(Color::from_hex(0xE8EEF8)).draw_text(
        font,
        &format!("Zebrano: {}", game.farm.harvested),
        Vec2::new(24.0, 24.0),
        24.0,
        TextAlign::Left,
    );
    ctx.gfx.color(Color::from_hex(0xB9C4D4)).draw_text(
        font,
        &format!("Puste {empty}  Rosnie {growing}  Gotowe {ready}"),
        Vec2::new(24.0, 54.0),
        18.0,
        TextAlign::Left,
    );

    // --- sterowanie (lewy dolny róg) ---
    ctx.gfx.color(Color::from_hex(0x8FA6C4)).draw_text(
        font,
        "WSAD chodzenie   mysz obrot   spacja skok   E sadz/zbierz   R nowa farma   F1 kolizje",
        Vec2::new(24.0, size.y - 32.0),
        16.0,
        TextAlign::Left,
    );

    // --- znacznik trybu debug (prawy dolny róg) ---
    //
    // Bez tego druty wyglądałyby jak błąd renderowania, a nie jak
    // włączone narzędzie. Kolor pasuje do pomarańczu kolidatorów.
    if game.show_colliders {
        ctx.gfx.color(Color::from_hex(0xFF7333)).draw_text(
            font,
            &format!("KOLIZJE ON  ({} brył)", game.world.boxes().len()),
            Vec2::new(size.x - 24.0, size.y - 32.0),
            16.0,
            TextAlign::Right,
        );
    }

    // --- wygrana ---
    if game.won {
        ctx.gfx
            .screen_space()
            .layer(101)
            .color(Color::from_hex(0x141A24))
            .draw_rect(Rect::new(
                Vec2::new(0.0, size.y * 0.5 - 60.0),
                Vec2::new(size.x, size.y * 0.5 + 60.0),
            ));
        ctx.gfx.color(Color::from_hex(0xF2C14E)).draw_text(
            font,
            "POLE DOJRZALE - ZBIERASZ!",
            Vec2::new(size.x * 0.5, size.y * 0.5 - 12.0),
            40.0,
            TextAlign::Center,
        );
        ctx.gfx.color(Color::from_hex(0xE8EEF8)).draw_text(
            font,
            "R - nowa farma",
            Vec2::new(size.x * 0.5, size.y * 0.5 + 26.0),
            20.0,
            TextAlign::Center,
        );
    }
}

/// Proxy renderera 3D podpinany do renderera 2D (patrz `uran-render`).
struct SceneProxy(Arc<Mutex<Renderer3d>>);

impl uran_render::Scene3d for SceneProxy {
    fn draw(&mut self, target: &uran_render::Scene3dTarget<'_>) {
        if let Ok(mut scene) = self.0.lock() {
            uran_render::Scene3d::draw(&mut *scene, target);
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if let Ok(mut scene) = self.0.lock() {
            uran_render::Scene3d::resize(&mut *scene, w, h);
        }
    }
}

/// Punkt wejścia.
fn main() {
    let game = Arc::new(Mutex::new(None::<Game>));

    let setup_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if guard.is_some() {
                return;
            }
            // JEDEN `Device` obsługuje oba renderery, a `samples` MUSI
            // się zgadzać — inaczej hdr-owa tekstura post-processingu
            // nie przejdzie walidacji wgpu.
            let built = {
                let r = ctx.renderer.as_ref().expect("renderer gotowy");
                Game::new(r.gpu(), r.format(), r.samples())
            };
            *guard = Some(built);
            let g = guard.as_mut().expect("właśnie utworzone");
            g.font = ctx.assets.load_font("assets/fonts/UranSans.ttf").ok();
            let proxy = Box::new(SceneProxy(Arc::clone(&g.scene)));
            if let Some(r) = ctx.renderer.as_mut() {
                r.set_scene3d(proxy);
            }
        }
    };

    let update_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let mut guard = game.lock().expect("lock gry");
            if let Some(g) = guard.as_mut() {
                update(ctx, g);
            }
        }
    };

    let draw_system = {
        let game = Arc::clone(&game);
        move |ctx: &mut Ctx| {
            let guard = game.lock().expect("lock gry");
            if let Some(g) = guard.as_ref() {
                draw_hud(ctx, g);
            }
        }
    };

    App::new()
        // Zrzut po 40 klatkach: tyle trzeba, żeby world mgły i cieni
        // się ustabilizowały. Używany w testach regresji wizualnej
        // (`FARM_SHOT=1 cargo run -p farm-simulator`).
        .screenshot(
            std::env::var("FARM_SHOT")
                .ok()
                .map(std::path::PathBuf::from),
            40,
        )
        .window(
            windowed(1280, 720)
                .title("Farm Simulator")
                .background(0x94BDE6)
                .vsync(true)
                .samples(1)
                // Kursor ukryty: w pierwszej osobie mysz obraca kamerę,
                // a wskazywanie w grze robi celownik rysowany w HUD.
                .cursor_visible(false)
                // ...i zablokowany w centrum okna. Bez `Locked` kursor
                // ucieka do rogów — przy 75° FOV i czułości 0,0035
                // kilka obrotów głowy wystarczy, żeby doszedł do
                // ściany okna i sterowanie umarło.
                .cursor_locked(true)
                // Pełny ekran, ale NIE przy zrzucie: `FARM_SHOT` potrzebuje
                // stałych 1280x720 do porównania piksel po pikselu, a
                // fullscreen wziąłby rozmiar monitora. Dlatego zrzut
                // świadomie rezygnuje z trybu pełnoekranowego.
                .fullscreen(std::env::var_os("FARM_SHOT").is_none()),
        )
        .add_startup_system(setup_system)
        .add_system(update_system)
        .add_system(draw_system)
        .run();
}
