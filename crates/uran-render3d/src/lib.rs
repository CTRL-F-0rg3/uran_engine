//! Renderer 3D — osobny podsystem, współdzielący z rendererem 2D tylko
//! `Device`/`Queue`/`Surface` (patrz `uran_render::scene3d`).
//!
//! Świadomie NIC tu nie dziedziczy po kodzie 2D: własne shadery, własny
//! depth buffer, własne potoki, własna kamera. Jedyna rzecz wspólna to
//! urządzenie GPU i obietnica klatki.
//!
//! # Styl oświetlenia
//!
//! Shader sceny realizuje model hybrydowy PBR + NPR, jaki widać w
//! gatunku „semi-realistic anime" (Arknights: Endfield, Mugen, Night
//! Abyss): warstwa bazowa dyfuzji przechodzi przez miękką rampę
//! rysunkową, a warstwa odbić zostaje w pełni fizyczna. Otoczenie
//! dostaje fizyczne niebo (Rayleigh + Mie), mgłę z rozpraszaniem Mie,
//! a post-processing składa z tego SSR, kontury ekranowe zanikające
//! w świetle, SSS, DoF, anamorficzny bloom, flarę i podział tonów.
//!
//! ## Jak to jest złożone
//!
//! ```text
//!   [pass cieni]  → głębokość z pozycji słońca
//!   [pass główny] → HDR (kolor) + G-Bufer (normalna + chropowatość)
//!                   + niebo jako pełnoekranowy trójkąt
//!   [kopia]       → głębokość do tekstury czytelnej przez postfx
//!   [postfx]      → god rays → SSR → bloom → kompozycja
//! ```

pub mod camera;
pub mod import;
pub mod material;
pub mod mesh;
pub mod postfx;
pub mod scene;
pub mod shadow;

pub use camera::{Camera3d, Ray};
pub use import::{ImportedModel, Material, MaterialLib, ModelPart};
pub use material::{MaterialBank, MaterialId, Surface};
pub use mesh::{box_corners, model_matrix, GpuMesh, InstanceModel, Mesh, SceneUniform, Vertex};
pub use postfx::{PostFx, PostSettings};
pub use scene::{Atmosphere, DrawCmd, Lighting, MeshId, Renderer3d, Stylization};
pub use shadow::{SceneBounds, ShadowMap, ShadowSettings, SHADOW_RES};

#[cfg(test)]
mod shader_tests {
    //! Testy parsujące WGSL.
    //!
    //! Shader jest zwykłym plikiem włączonym przez `include_str!`, więc
    //! literówka w nim nie zatrzymuje kompilacji Rusta — wgpu zgłosi błąd
    //! dopiero przy pierwszym uruchomieniu gry, już po zbudowaniu okna
    //! i załadowaniu assetów. Te testy przenoszą wykrywanie błędów na
    //! `cargo test`, gdzie nie ma GPU i nie trzeba okna.

    use naga::valid::{Capabilities, ValidationFlags, Validator};

    /// Parsuje i waliduje WGSL, zwracając błędy jako tekst.
    ///
    /// Zwraca `Err` z pełnym komunikatem naga, żeby test pokazał nie
    /// tylko „coś jest nie tak", ale gdzie — naga dokleja do komunikatu
    /// numer linii i kolumnę.
    fn validate(label: &str, source: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(source)
            .map_err(|e| format!("{label}: błąd parsowania WGSL:\n{e:?}"))?;
        Validator::new(
            ValidationFlags::all(),
            // `Capabilities::default()` = pusty zestaw, czyli „nic
            // dodatkowego nie jest włączone". To najostrzejsza
            // konfiguracja: wychwyci użycie rozszerzeń, których
            // sterownik może nie mieć.
            Capabilities::default(),
        )
        .validate(&module)
        // naga 0.19 zwraca tu `Result<ModuleInfo, WithSpan<...>>`.
        // `ModuleInfo` nas nie interesuje, więc odrzucamy wynik —
        // interesuje nas tylko wariant błędu.
        .map(|_| ())
        .map_err(|e| format!("{label}: błąd walidacji WGSL:\n{e:?}"))
    }

    #[test]
    fn scene_shader_parses() {
        let src = include_str!("s3d.wgsl");
        if let Err(e) = validate("s3d.wgsl", src) {
            panic!("{e}");
        }
    }

    #[test]
    fn post_shader_parses() {
        let src = include_str!("post.wgsl");
        if let Err(e) = validate("post.wgsl", src) {
            panic!("{e}");
        }
    }

    /// Sprawdza, że oba potoki, których używa `scene.rs`, istnieją
    /// w shaderze sceny pod dokładnie tymi nazwami.
    ///
    /// To chroni przed scenariuszem, w którym ktoś zmieni nazwę
    /// entry pointu w WGSL, a `scene.rs` zostanie stary — wtedy
    /// wgpu odrzuci potok z komunikatem, który nie mówi, o jaką
    /// nazwę chodzi.
    #[test]
    fn scene_shader_exposes_expected_entry_points() {
        let src = include_str!("s3d.wgsl");
        for entry in [
            "fn vs_main",
            "fn vs_shadow",
            "fn vs_sky",
            "fn fs_main",
            "fn fs_sky",
            // Skinning. Te dwa muszą istnieć obok `vs_main`, bo potok
            // postaci różni się wyłącznie punktem wejścia — `fs_main`
            // jest wspólny, więc postać oświetla się dokładnie tak
            // jak bryły.
            "fn vs_skin",
            "fn vs_skin_shadow",
        ] {
            assert!(
                src.contains(entry),
                "s3d.wgsl nie zawiera punktu wejścia `{entry}`"
            );
        }
    }

    /// To samo dla potoków post-processingu.
    #[test]
    fn post_shader_exposes_expected_entry_points() {
        let src = include_str!("post.wgsl");
        for entry in [
            "fn vs_fullscreen",
            "fn fs_bright",
            "fn fs_blur",
            "fn fs_ssr",
            "fn fs_godray",
            "fn fs_ssao",
            "fn fs_ao_blur",
            "fn fs_composite",
        ] {
            assert!(
                src.contains(entry),
                "post.wgsl nie zawiera punktu wejścia `{entry}`"
            );
        }
    }

    /// Każde `@group(N) @binding(M)` w shaderze ma swój wpis w layoucie.
    ///
    /// `wgpu` waliduje to dopiero przy `create_bind_group`, ale błąd
    /// jest wtedy nieczytelny („binding not found"). Ten test porównuje
    /// deklaracje WGSL z layoutami w kodzie Rust.
    #[test]
    fn post_bindings_match_layout_entries() {
        let src = include_str!("post.wgsl");
        // Layout z `postfx.rs`: uniform, src, bloom, sampler,
        // gbuffer, depth, ssr, rays, ao.
        let expected = [0u32, 1, 2, 3, 4, 5, 6, 7, 8];
        for b in expected {
            let needle = format!("@binding({b})");
            assert!(
                src.contains(&needle),
                "post.wgsl nie deklaruje bindingu {b}, a layout go wymaga"
            );
        }
    }

    /// `Surface::base` musi docierać do shadera.
    ///
    /// Bez tego `fs_main` brał kolor wyłącznie z wierzchołka, więc
    /// programowy materiał (`Surface::skin`, `Surface::metal`) był
    /// po cichu ignorowany. Test pilnuje obu rzeczy naraz: że pole
    /// istnieje w uniformie i że `mix` po nim sięga.
    #[test]
    fn material_base_color_reaches_the_shader() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("base: vec4<f32>"),
            "s3d.wgsl: struktura Material nie ma pola `base`"
        );
        assert!(
            src.contains("mat.base.rgb"),
            "s3d.wgsl: fs_main nie używa `mat.base.rgb`, więc kolor \
             materiału z kodu nigdy nie trafi do piksela"
        );
        // Import `.obj` wkleja `Kd` do wierzchołka, więc `base.w`
        // musi wybierać między tymi dwoma źródłami, a nie mnożyć je.
        assert!(
            src.contains("mix(in.color, mat.base.rgb, mat.base.w)"),
            "s3d.wgsl: baza musi być wybierana flagą, nie mnożona"
        );
    }

    /// Skinning musi mieć swój własny binding kości — w grupie 2.
    ///
    /// Gdyby macierz kości leżała w grupie 0 (wspólnej z `models`),
    /// każda statyczna bryła świata musiałaby dostać sztuczny bufor
    /// na 73 macierze. Osobna grupa sprawia, że `DrawCmd` dla ścian
    /// w ogóle jej nie dotyka.
    #[test]
    fn skinning_ma_wlasny_binding_kosci() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("@group(2) @binding(0) var<storage, read> bones"),
            "s3d.wgsl nie deklaruje kości w grupie 2"
        );
        // Wagi muszą być normalizowane w shaderze: plik gwarantuje sumę
        // ~1, ale eksporterzy bywają niedokładni, a nieznormalizowane
        // wagi dają ciemne smugi na krawędziach skóry.
        assert!(
            src.contains("normalized_weights"),
            "s3d.wgsl nie normalizuje wag skinningu"
        );
    }

    /// Efekty nie mogą być agresywne **domyślnie**.
    ///
    /// Ustawienia domyślne są tym, co dostaje demo, które niczego nie
    /// ustawia — a większość ich nie ustawia. Za agresywne wartości
    /// sprawiają, że obraz wygląda „chory" (anamorficzne kreski, szare
    /// plamy AO, plamiste niebo od bloomu) zamiast atrakcyjny.
    ///
    /// Test jest celowo wartościowy, a nie „mniejszy niż poprzednio":
    /// gdyby ktoś podniósł `ssao_strength` do 0.9, test by zadal
    /// pytanie „po co", zamiast przechodzić po cichu.
    #[test]
    /// Ostatnie wartości dobrane odręcznie — traktowane jako kontrakt.
    ///
    /// Test `domyslne_efekty_sa_lagodne` pilnuje tylko górnych progów
    /// („nie za mocno"). Ten pilnuje dokładnych liczb, bo efekt, o który
    /// chodziło, brzmiał wprost: obraz był mocno przepłacony efektami
    /// i każde dalsze podbicie któregokolwiek z tych parametrów wraca
    /// do tego samego problemu.
    ///
    /// Wyjątek: `dof_max_blur` ma minimalne, ale niezerowe granice —
    /// przy 0.0 shader dzieli przez `max(d, 0.001)`, co daje nieskończony
    /// blur na pikselach w płaszczyźnie ogniskowania.
    /// Ambient musi być **dużo mniejszy** od natężenia słońca.
    ///
    /// Obraz wychodził „jak przez mętną soczewkę" właśnie dlatego, że
    /// `ambient` wynosił ~0.4 przy słońcu 4.0. Stosunek poniżej ~0.05
    /// daje realny kontrast: strona w cieniu jest ciemna, a nie tylko
    /// „inny odcień tego samego jasnego koloru".
    #[test]
    fn ambient_nie_wypelnia_cieni() {
        let l = crate::scene::Lighting::default();
        let ambient_luma: f32 =
            l.ambient[0] * 0.2126 + l.ambient[1] * 0.7152 + l.ambient[2] * 0.0722;
        let sun_luma: f32 =
            l.light_color[0] * 0.2126 + l.light_color[1] * 0.7152 + l.light_color[2] * 0.0722;
        let ratio = ambient_luma / (sun_luma * l.intensity);

        assert!(
            ratio < 0.05,
            "ambient:słońce = {:.3} — cienie wypełnione (prawidłowo < 0.05)",
            ratio
        );
        // Cienie nadal muszą być widoczne jako „niebo z wnętrza",
        // a nie czysta czerń — inaczej scena wygląda jak wyłączony
        // silnik, a nie jak niedoświetlone pomieszczenie.
        assert!(
            ambient_luma > 0.01,
            "ambient = {} — zbyt ciemno, cienie staną się dziurami",
            ambient_luma
        );
    }

    /// Kalibracja obrazu musi być aktywna domyślnie.
    ///
    /// Te trzy pola odpowiadają na zgłoszone problemy: mętna soczewka
    /// (ostrość), szare cienie (lift), brak różnicy między materiałami
    /// (chropowatość). Wyłączone zostawiają obraz dokładnie takim,
    /// jak był przed poprawką.
    #[test]
    fn kalibracja_obrazu_jest_wlaczona() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();

        // 0.55 -> 0.12. Wysoka ostrosc dawala biale obwodki
        // (fringes) na kazdej krawedzi sylwetki, co czytalo sie
        // jako "dziwne kolory" mimo poprawnej palety. Nadal aktywna,
        // ale wyraznie slabsza.
        assert!(
            p.clarity > 0.05,
            "clarity = {} - obraz bedzie znowu metny",
            p.clarity
        );
        // Powyżej 1.5 unsharp daje halo (białą obwódkę) na
        // sylwetkach, a to dokładnie ten defekt, który usuwamy.
        assert!(
            p.clarity <= 1.5,
            "clarity = {} — pojawi się halo na krawędziach",
            p.clarity
        );
        assert!(
            p.shadow_lift > 0.0 && p.shadow_lift <= 0.05,
            "shadow_lift = {} — poza zakresem 0..=0.05",
            p.shadow_lift
        );
        // `roughness_bias` celowo 0: korekta chropowatości to decyzja
        // sceny, a nie silnika. Test pilnuje tylko, że pole istnieje
        // i mieści się w zadeklarowanym zakresie.
        assert!(
            (-1.0..=1.0).contains(&p.roughness_bias),
            "roughness_bias = {} poza -1..1",
            p.roughness_bias
        );
    }

    /// Nasycenie i kontrast muszą być wyżej niż 1.
    ///
    /// AgX celowo odbarwia kolory (żeby światła nie były plamami), więc
    /// domyślne 1.0 dawało wyblakłą scenę. Podbicie po tonemapie
    /// odzyskuje intensywność materiałów.
    #[test]
    fn kolory_sa_nasycone_a_ma_kontrast() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();
        assert!(
            p.saturation > 1.05,
            "saturation = {} — scena wygląda wyblakła",
            p.saturation
        );
        assert!(
            p.contrast > 1.05,
            "contrast = {} — brak głębi tonalnej",
            p.contrast
        );
    }

    /// Macierze AgX muszą być transponowane, a tonemap musi mieć
    /// parę inset/outset **oraz** konwersję Rec.2020.
    ///
    /// ## Dlaczego to pilnujemy testem
    ///
    /// Wartości macierzy AgX są wypisane **wierszami**, a WGSL (jak
    /// GLSL) buduje macierz z **kolumn**. Bez `transpose()` macierz
    /// wychodzi transponowana, a wynik — przesunięty w odcieniu.
    ///
    /// ## Konwersja Rec.2020 — nie ozdoba
    ///
    /// AgX zaprojektowano na primariesach **Rec.2020**. Referencja
    /// (Filament / Blender 4 / three.js) ma więc cztery etapy:
    /// `sRGB -> Rec.2020`, `inset`, `log2`/sigmoid, `outset`,
    /// `pow(2.2)`, `Rec.2020 -> sRGB`, `clamp`.
    ///
    /// Wcześniejsza wersja ZAMIAST dwóch macierzy Rec.2020 używała
    /// własnych `agx_transform` / `agx_transform_inv`, których w AgX
    /// nie ma. Obraz pozostawał „prawie dobry" — to najgorsza cecha
    /// tego błędu, bo cichy. Objaw: trwały przesmyk w magenta na
    /// jasnych i nasyconych partiach, zgłoszony jako „różowe słońce".
    #[test]
    fn agx_ma_transpozycje_i_inset_outset() {
        let src = include_str!("post.wgsl");

        for fname in [
            "linear_srgb_to_linear_rec2020",
            "linear_rec2020_to_linear_srgb",
            "agx_inset",
            "agx_outset",
        ] {
            let start = src
                .find(&format!("fn {fname}("))
                .unwrap_or_else(|| panic!("brak funkcji {fname} w post.wgsl"));
            let body = &src[start..(start + 400).min(src.len())];
            assert!(
                body.contains("transpose("),
                "{fname}() nie używa transpose() — wartości AgX są wierszami, \
                 a WGSL buduje macierz z kolumn. Efekt: przesmyk w odcieniu \
                 na jasnych partiach (tarcza słońca wychodzi różowa)."
            );
        }

        // Śmieciowe macierze rektyfikacji musiały zniknąć: ich bardzo
        // podobne współczynniki (0.842479…) kuszą do przywrócenia.
        assert!(
            !src.contains("fn agx_transform("),
            "agx_transform() wróciła — w referencyjnym AgX nie ma takiej \
             macierzy; jej rolę pełnią konwersje Rec.2020"
        );
        assert!(
            !src.contains("fn agx_transform_inv("),
            "agx_transform_inv() wróciła — j.w."
        );

        // Para inset/outset musi być UŻYTA w tonemapie, nie tylko
        // zdefiniowana. Bez insetu jasne, nasycone barwy przesuwają się
        // w magenta.
        let tm_start = src
            .find("fn agx_tonemap(")
            .expect("brak agx_tonemap w post.wgsl");
        let tm = &src[tm_start..(tm_start + 1200).min(src.len())];
        assert!(
            tm.contains("linear_srgb_to_linear_rec2020() *"),
            "agx_tonemap() nie wchodzi w Rec.2020 — bez tego AgX miesza \
             gamut sRGB z gamutem, na którym został zaprojektowany"
        );
        assert!(
            tm.contains("agx_inset() * val"),
            "agx_tonemap() nie stosuje insetu przed log2"
        );
        assert!(
            tm.contains("agx_outset() * val"),
            "agx_tonemap() nie stosuje outsetu po krzywej tonalnej"
        );
        assert!(
            tm.contains("linear_rec2020_to_linear_srgb() * val"),
            "agx_tonemap() nie wraca do sRGB — wynik pozostanie w Rec.2020 \
             i będzie przesunięty w odcieniu"
        );

        // Kolejność ma znaczenie: wejście PRZED insetem, wyjście PO
        // outsetcie. Sprawdzamy pozycje, bo sama obecność nie wystarczy.
        let rec_in = tm
            .find("linear_srgb_to_linear_rec2020() *")
            .expect("wejście do Rec.2020");
        let inset = tm.find("agx_inset() * val").expect("inset");
        let log = tm.find("log2(").expect("log2");
        let outset = tm.find("agx_outset() * val").expect("outset");
        let rec_out = tm
            .find("linear_rec2020_to_linear_srgb() * val")
            .expect("wyjście z Rec.2020");
        assert!(
            rec_in < inset && inset < log && log < outset && outset < rec_out,
            "kolejność AgX jest zła: oczekiwane sRGB->Rec2020, inset, log2, \
             outset, Rec2020->sRGB (indeksy: {rec_in}, {inset}, {log}, \
             {outset}, {rec_out})"
        );
    }

    /// Mgła domyślna musi być **delikatna** i nie rozjaśniać dystansu.
    ///
    /// ## Dlaczego
    ///
    /// Zgłoszone na zrzucie: „wszystko jest mętne". Przy gęstości
    /// 0.0045 mgła zaczynała zacierać obraz kilkanaście metrów od
    /// kamery, a horyzont znikał w jednolitej, jasnej błękitnej ścianie.
    ///
    /// Test pilnuje dwóch rzeczy naraz: małej gęstości **oraz**
    /// tego, że kolor mgły jest ciemniejszy od nieba. Ten drugi warunek
    /// jest ważniejszy, niż się wydaje — zbyt jasna mgła nie tyle
    /// ukrywa dystans, co **rozjaśnia** go, przez co cały kadr ciągnie
    /// ku blademu błękitowi i traci nasycenie.
    #[test]
    fn mgla_domyslna_jest_delikatna_i_nie_rozjasnia_dystansu() {
        let a = crate::scene::Atmosphere::default();

        // 1 - e^(-0.0016 * 100 m) = 15%: ledwie zauważalne zamglenie.
        // 0.0045 dawało 36%, co na zrzucie było już „mętne".
        let at_100m = 1.0 - (-a.fog_density * 100.0).exp();
        assert!(
            at_100m < 0.20,
            "fog_density = {} daje {:.0}% mgły na 100 m — obraz będzie mętny \
             (prawidłowo < 20%)",
            a.fog_density,
            at_100m * 100.0
        );

        // Kolor mgły musi być ciemniejszy od nieba w typowej scenie
        // (`endfield-3d` ustawia 0.34/0.47/0.65).
        let fog_luma = a.fog_color[0] * 0.2126 + a.fog_color[1] * 0.7152 + a.fog_color[2] * 0.0722;
        let sky_luma = 0.34 * 0.2126 + 0.47 * 0.7152 + 0.65 * 0.0722;
        assert!(
            fog_luma < sky_luma,
            "mgła (luma {:.2}) nie jest ciemniejsza od nieba ({:.2}) — dystans \
             będzie się rozjaśniał zamiast zanikać",
            fog_luma,
            sky_luma
        );
    }

    /// Demo nie może zalewać otoczenia ambientem.
    ///
    /// ## Dlaczego to osobny test, a nie część `ambient_nie_wypelnia_cieni`
    ///
    /// Tamten pilnuje wartości domyślnej w `Lighting::default()`. Ten
    /// pilnuje tego, czego tamten nie widzi: **demo nadpisujące ambient
    /// własną, większą wartością**. To właśnie one psuły obrazy —
    /// `farm-simulator` miało 0.36/0.45/0.60 przy słońcu 4.0, czyli
    /// ambient jaśniejszy niż znaczna część strony na słońcu. Kadr był
    /// płaski, a kolory (trawa, drewno) wychodziły „plastikowe".
    ///
    /// Test czyta pliki demo jako tekst, bo te ustawienia nie żyją
    /// w bibliotece.
    #[test]
    fn demo_nie_zalewa_otoczenia_ambientem() {
        // Oczekiwany ambient i natężenie słońca w każdym demo.
        // `endfield-3d` ma słońce 6.5 (celowo mocniejsze), więc jego
        // proporcja i tak jest ~20× mniejsza niż w `Lighting::default()`.
        //
        // `include_str!` liczy ścieżkę względem pliku `lib.rs`, czyli
        // `crates/uran-render3d/src/`. Do katalogu głównego repo
        // są trzy poziomy w górę. Ścieżki są literałami, bo `concat!`
        // nie przyjmuje wyrażeń.
        for (path, src, expected, intensity) in [
            (
                "farm-simulator",
                include_str!("../../../farm-simulator/src/main.rs"),
                "[0.13, 0.16, 0.21]",
                4.0f32,
            ),
            (
                "junak-rider",
                include_str!("../../../junak-rider/src/main.rs"),
                "[0.13, 0.16, 0.21]",
                4.0f32,
            ),
            (
                "endfield-3d",
                include_str!("../../../endfield-3d/src/main.rs"),
                "[0.15, 0.19, 0.26]",
                6.5f32,
            ),
        ] {
            assert!(
                src.contains(&format!("l.ambient = {expected};")),
                "{path}: brak oczekiwanego ambientu {expected}. Zbyt wysoki \
                 ambient zalewa kadr i zabija kontrast między słońcem a cieniem."
            );

            // Proporcja: luminancja ambientu / (luminancja słońca *
            // natężenie) — ta sama miara, której używa
            // `ambient_nie_wypelnia_cieni`.
            let amb: Vec<f32> = expected
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .filter_map(|v| v.trim().parse::<f32>().ok())
                .collect();
            let ambient_luma = amb[0] * 0.2126 + amb[1] * 0.7152 + amb[2] * 0.0722;
            // Słońce [1.0, 0.93, 0.80] — typowe dla tego silnika.
            let sun_luma = 1.0 * 0.2126 + 0.93 * 0.7152 + 0.80 * 0.0722;
            let ratio = ambient_luma / (sun_luma * intensity);
            assert!(
                ratio < 0.05,
                "{path}: ambient:słońce = {ratio:.3} — cienie wypełnione \
                 (prawidłowo < 0.05)"
            );
        }
    }

    /// Regresja: `ndotl` nie moze byc liczony dwa razy.
    ///
    /// `fs_main` liczyl `diffuse = kd * albedo * ramp / PI`, a potem
    /// `direct = (diffuse + specular) * sun * ndotl` mnozyl to jeszcze
    /// raz przez `ndotl`. Dyfuzja byla wiec kwadratem cosinusa, a
    /// `pbr.wgsl` tego nie robi - tam waga dyfuzji jest czystym
    /// `albedo / PI`, a `n_dot_l` mnozy caly termin w petli.
    ///
    /// Skutek byl widoczny gołym okiem: obraz plaski, cienie zlane
    /// z oswietleniem i kolory niezgodne z materialem.
    #[test]
    fn dyfuzja_nie_jest_mnozona_dwa_razy_przez_ndotl() {
        let src = include_str!("s3d.wgsl");

        // `ndotl` moze wystapic w definicjach (`let ndotl = ...`) i w
        // miejscach, gdzie jest poprawny (odbicie, SSS, rim), wiec
        // szukamy konkretnie wzorca, ktory byl bledny.
        assert!(
            !src.contains("(diffuse + specular) * sun * ndotl"),
            "s3d.wgsl: dyfuzja znowu mnozona przez ndotl razem z odbiciem. \
             Waga rampy musi byc w `diffuse`, a `ndotl` moze dotyczyc \
             tylko odbicia."
        );

        // I na odwrot: poprawne rozdzielenie musi istniec, inaczej
        // powyzszy test przejdzilby na pustym shaderze.
        assert!(
            src.contains("diffuse * sun + specular * sun * ndotl"),
            "s3d.wgsl: brak rozdzielenia wagi dyfuzji i odbicia. \
             Oczekiwane `diffuse * sun + specular * sun * ndotl`."
        );

        // `ndot_final` jest kompletna waga dyfuzji, wiec nie wolno
        // mnozyc go jeszcze raz w `direct`.
        assert!(
            !src.contains("* ndl_final *"),
            "s3d.wgsl: `ndl_final` uzyty jako dodatkowy czynnik. \
             To jest juz kompletna waga dyfuzji."
        );
    }

    /// `MIN_ROUGHNESS` musi realnie chronic przed rozpryskiem GGX.
    ///
    /// Gdy `alpha → 0`, `D → ∞`, a pojedynczy piksel odbicia zamienia
    /// sie w bialy punkt migoczacy przy ruchu kamery. Sufit z
    /// `pbr.wgsl` (0.045) jest tu wzorcem.
    #[test]
    fn minimalna_chropowatosc_jest_zastosowana() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("const MIN_ROUGHNESS: f32 = 0.045;"),
            "s3d.wgsl: brak stalej MIN_ROUGHNESS = 0.045"
        );
        assert!(
            src.contains("MIN_ROUGHNESS, 1.0)"),
            "s3d.wgsl: MIN_ROUGHNESS nie jest uzyte w clamp() chropowatosci"
        );
    }

    /// Specular AA musi liczyc splot z pochodnych (`dpdx`/`dpdy`).
    ///
    /// Bez tego gladkie materiale migocza przy ruchu kamery - to
    /// najbardziej widoczny artefakt po przeniesieniu reszty fizyki
    /// z `pbr.wgsl`.
    #[test]
    fn specular_aa_liczy_splot_z_pochodnych() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("dpdx(N)") && src.contains("dpdy(N)"),
            "s3d.wgsl: brak `dpdx(N)` / `dpdy(N)` - specular AA nie dziala"
        );
        assert!(
            src.contains("specular_kernel"),
            "s3d.wgsl: brak jadra specular AA (`specular_kernel`)"
        );
    }

    #[test]
    fn domyslne_sa_dokladnie_takie_jak_zamowiono() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();

        assert_eq!(p.ssao_strength, 0.10, "sila SSAO");
        assert_eq!(p.ssao_radius, 0.10, "promien SSAO w metrach");
        assert_eq!(p.outline_strength, 0.02, "kontur");
        assert_eq!(p.ssr_strength, 0.11, "odbicie ekranowe");
        assert_eq!(p.dof_max_blur, 0.10, "maks. blur DoF");
        // Kalibracja obrazu: to sa trzy wartosci, ktorymi
        // wczesniej zjadlym kolory. `clarity` 0.55 dawal biale
        // obwodki na krawedziach, `shadow_lift` 0.03 szarzylo
        // cienie, a `split_tone` 0.35 przesuwal cala palete.
        assert_eq!(p.clarity, 0.12, "ostrosc");
        assert_eq!(p.shadow_lift, 0.01, "lift cieni");
        assert_eq!(p.split_tone, 0.10, "podzial tonow");
    }

    /// Wszystkie cztery powyższe wartości muszą być **bardzo małe** —
    /// to efekt świadomie wyłączony, nie tylko osłabiony.
    ///
    /// Rozróżnienie jest ważne przy diagnostyce: jeśli AO znika
    /// całkowicie, to oczywista przyczyna to zbyt mały `ssao_strength`
    /// albo `ssao_radius`, a nie błąd w shaderze.
    #[test]
    fn cztery_efekty_sa_zasadniczo_wylaczone() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();
        assert!(p.ssao_strength <= 0.25, "AO nadal aktywne");
        assert!(p.ssao_radius <= 0.15, "promień AO zbyt duży");
        assert!(p.outline_strength <= 0.05, "kontur nadal aktywny");
        assert!(p.ssr_strength <= 0.15, "SSR nadal aktywne");
        assert!(p.dof_max_blur <= 0.5, "DoF nadal aktywny");
    }

    #[test]
    fn domyslne_efekty_sa_lagodne() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();

        // AO: najbardziej szkodliwy przy dużym promieniu — zaciemnia
        // cały obrys obiektu, a nie sam styk z podłożem.
        assert!(
            p.ssao_strength <= 0.6,
            "ssao_strength = {}",
            p.ssao_strength
        );
        assert!(p.ssao_radius <= 0.4, "ssao_radius = {}", p.ssao_radius);
        assert!(
            p.ssao_intensity <= 1.3,
            "ssao_intensity = {}",
            p.ssao_intensity
        );

        // Kontur powyżej ~0.4 zamienia postać w plakat z grubą ramą.
        assert!(
            p.outline_strength <= 0.4,
            "outline = {}",
            p.outline_strength
        );

        // Anamorficzna poświata powyżej 0.6 daje poziome kreski
        // na każdej jasnej plamie — obraz wygląda „chory".
        assert!(p.anamorphic <= 0.25, "anamorphic = {}", p.anamorphic);
        assert!(p.lens_flare <= 0.20, "lens_flare = {}", p.lens_flare);
        assert!(p.god_rays <= 0.30, "god_rays = {}", p.god_rays);

        // Bloom 0.32+ przy progu 1.15 świecił na każdej jasnej
        // powierzchni zamiast tylko na źródłach.
        assert!(p.bloom <= 0.20, "bloom = {}", p.bloom);

        // Winieta powyżej 0.25 przyciemnia narożnik jak defekt kadru.
        assert!(p.vignette <= 0.20, "vignette = {}", p.vignette);

        // Split tone powyżej 0.4 przesuwa całą paletę w błękit/bursztyn.
        assert!(p.split_tone <= 0.40, "split_tone = {}", p.split_tone);

        // Antyaliasing to NIE efekt — obniżanie dodaje schodki.
        assert!(
            p.antialias >= 0.5,
            "antialias = {} — to nie efekt",
            p.antialias
        );
    }

    /// Odrzucanie próbek AO musi działać domyślnie.
    ///
    /// `0` oznaczałoby całkowite wyłączenie odrzucania, a wtedy
    /// `ssao_reject_fadeoff` cicho nic nie robiłby, a `fs_ssao`
    /// zachowywałby się jak stary test kulowy.
    #[test]
    fn odrzucanie_ao_ma_sensowna_domyslna_wartosc() {
        use crate::postfx::PostSettings;
        let p = PostSettings::default();
        assert!(
            (50.0..=200.0).contains(&p.ssao_reject_fadeoff),
            "ssao_reject_fadeoff = {} — poza zakresem użytecznym 50..=200",
            p.ssao_reject_fadeoff
        );
    }

    /// Uniform musi przenosić `reject_fadeoff` do shadera.
    ///
    /// Parametr jedzie w `rays.z` (pole rezerwowe), bo dopisanie
    /// nowego `vec4` przesunęłoby wszystkie późniejsze pola.
    #[test]
    fn uniform_oddaje_parametr_do_pola_rays_z() {
        use crate::postfx::{PostParams, PostSettings};
        let s = PostSettings::default();
        let p = PostParams::from_settings(
            &s,
            /* w */ 1920.0,
            /* h */ 1080.0,
            /* time */ 0.0,
            /* sun */ [0.5; 4],
            /* near */ 0.1,
            /* far */ 1000.0,
            /* focus */ 1.0,
            /* tan_half_fov */ 0.5,
            /* aspect */ 16.0 / 9.0,
        );
        assert_eq!(
            p.rays[2], s.ssao_reject_fadeoff,
            "rays.z musi nieść ssao_reject_fadeoff dla `fs_ssao`"
        );
    }

    /// Mgła musi liczyć **analityczny całkowity** gęstości, a nie
    /// `1 - exp(-dist · ρ)`.
    ///
    /// ## Dlaczego to pilnujemy testem
    ///
    /// Wersja liniowa daje jednolitą, „mleczną" warstwę na każdej
    /// wysokości — dach domu tonął w mgle tak samo jak ulica pod nim.
    /// WickedEngine liczy to analitycznie (`fogHF.hlsli`), więc powtarzamy
    /// u siebie ten sam wzór, żeby zachować zgodny wygląd.
    #[test]
    fn mgla_jest_wysokosciowa_a_nie_jednolita() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("base_distance") && src.contains("exp_distance"),
            "s3d.wgsl nie ma analitycznej całkowitej mgły (brak rozdzielenia \
             na część bazową i eksponencjalną)"
        );
        // Stary, jednolity wzór może istnieć TYLKO jako wypadek przy
        // `fog.x == 0`. Poza tym nie powinien się pojawiać.
        assert!(
            src.contains("fog_amount = 1.0 - exp(-dist * scene.atmos.x)"),
            "s3d.wgsl nie ma nawet przypadku zapasowego dla mgły jednolitej"
        );
        // Anizotropia rozpraszania musi być z parametrizowana — mgła
        // gruntowa i niebo to różne ośrodki o różnej fazie.
        assert!(
            src.contains("henyey_greenstein(dot(-ray_dir, L), FOG_PHASE_G)"),
            "faza mgły musi liczyć kąt z kierunku promienia, nie z `V` \
             (kierunek do oka) — inaczej mgła świeci w złym miejscu"
        );
    }

    /// Rozmiar uniformu sceny musi zgadzać się z WGSL.
    ///
    /// `fog: vec4<f32>` doszło do struktury `Scene`, więc uniform urosnął
    /// o 16 B. Test chroni przed sytuacją, w której Rust i shader
    /// rozjeżdżają się cicho, a wgpu odrzuci bind group dopiero
    /// podczas renderowania.
    #[test]
    fn uniform_sceny_ma_miejsce_na_mgle() {
        let src = include_str!("s3d.wgsl");
        assert!(
            src.contains("fog: vec4<f32>"),
            "s3d.wgsl nie deklaruje pola `fog`"
        );
        assert_eq!(
            std::mem::size_of::<crate::mesh::SceneUniform>() % 16,
            0,
            "SceneUniform musi być wielokrotnością 16 B dla WGSL"
        );
    }

    /// Rozmiar uniformu materiału musi zgadzać się z obiektem w WGSL.
    #[test]
    fn material_uniform_is_six_vec4s() {
        assert_eq!(
            std::mem::size_of::<crate::material::MaterialUniform>(),
            96,
            "MaterialUniform musi mieć 6 × vec4, bo WGSL ma 6 pól vec4"
        );
    }
}
