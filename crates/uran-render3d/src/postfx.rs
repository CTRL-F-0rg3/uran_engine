//! Post-processing: AA, bloom, tonemapping, aberracja, winieta, ziarno.
//!
//! Celowo ODSEPAROWANY moduł zamiast rozbudowy `Renderer3d`. Trzyma
//! własne tekstury, potoki i bind grupy, a `Renderer3d` rozmawia z nim
//! tylko przez trzy metody: `hdr_view`, `composite` i `invalidate`.
//!
//! Dzięki temu reszta renderera 3D pozostaje nietknięta — warunek,
//! żeby dało się to wdrożyć małymi krokami.

use uran_render::Scene3dTarget;

/// Parametry post-processingu (UKŁAD ZGODNY Z `Params` W `post.wgsl`).
///
/// 11 * `vec4` = 176 B. Wszystko `vec4`, bo WGSL wyrównuje `vec3` do 16 B.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PostParams {
    /// `x` = ekspozycja, `y` = siła bloom, `z` = winieta, `w` = aberracja.
    pub grade: [f32; 4],
    /// `x` = próg bloom, `y` = ziarno, `z` = saturacja, `w` = kontrast.
    pub fx: [f32; 4],
    /// `x,y` = rozmiar kadru, `z` = czas, `w` = siła AA.
    pub screen: [f32; 4],
    /// `x,y` = słońce w UV, `z` = widoczność (0 = poza kadrem),
    /// `w` = rezerwa.
    pub sun: [f32; 4],
    /// `x` = siła konturu, `y` = grubość px, `z` = próg, `w` = zanikanie.
    pub outline: [f32; 4],
    /// `x` = siła SSR, `y` = maks. dystans, `z` = grubość,
    /// `w` = maks. chropowatość.
    pub ssr: [f32; 4],
    /// `x` = siła SSS, `y` = promień px, `z` = siła DoF, `w` = fokus.
    pub scatter: [f32; 4],
    /// `x` = apertura, `y` = maks. blur px, `z` = anamorficzna siła,
    /// `w` = siła flary.
    pub lens: [f32; 4],
    /// `x` = siła god rays, `y` = gęstość,
    /// `z` = odrzucanie próbek AO (patrz `ssao_reject_fadeoff`),
    /// `w` = rezerwa.
    pub rays: [f32; 4],
    /// `x,y` = near, far do liniaryzacji głębokości.
    pub depth: [f32; 4],
    /// `x` = siła podziału tonów, `y,z,w` = rezerwa na przyszłe
    /// krzywe (lift/gamma/gain).
    pub grade2: [f32; 4],
    /// `x` = siła SSAO, `y` = promień w jednostkach świata,
    /// `z` = bias (przesunięcie próbki wzdłuż normalnej),
    /// `w` = intensywność (potęga zaciennienia).
    pub ssao: [f32; 4],
    /// Dane projekcji potrzebne do odtworzenia pozycji w przestrzeni
    /// oka: `x` = tan(FOV/2) w pionie, `y` = aspect, `z,w` = rezerwa.
    ///
    /// Bez tego SSAO nie potrafi zbudować promienia przez piksel, więc
    /// nie policzyłby odległości w METRACH — dostałby wartości w
    /// pikselach, czyli takie zacienienie zależałoby od rozdzielczości
    /// okna i rozjeżdżałoby się przy zmianie rozmiaru.
    pub proj: [f32; 4],
}

impl PostParams {
    /// Buduje uniform głównego passu z ustawień i danych kadru.
    ///
    /// ## Po co to wydzielone
    ///
    /// Funkcja jest czysta — nie dotyka GPU. Wcześniej budowa uniformu
    /// siedziała w `upload_params`, więc jedyny sposób sprawdzenia, czy
    /// `ssao_reject_fadeoff` wylądował w `rays.z`, był uruchomienie
    /// gry i zobaczenie AO. Teraz test robi to bez okna i bez karty.
    #[allow(clippy::too_many_arguments)]
    pub fn from_settings(
        s: &PostSettings,
        w: f32,
        h: f32,
        time: f32,
        sun: [f32; 4],
        near: f32,
        far: f32,
        focus: f32,
        tan_half_fov: f32,
        aspect: f32,
    ) -> Self {
        Self {
            grade: [s.exposure, s.bloom, s.vignette, s.chromatic],
            fx: [s.bloom_threshold, s.grain, s.saturation, s.contrast],
            screen: [w, h, time, s.antialias],
            sun,
            outline: [
                s.outline_strength,
                s.outline_width,
                s.outline_threshold,
                s.outline_fade,
            ],
            ssr: [
                s.ssr_strength,
                // `max_dist` nie jest używany przez shader (promień
                // kończy się na krawędzi kadru), ale zostawiamy pole,
                // żeby układ uniformu nie rozjechał się z WGSL.
                80.0,
                s.ssr_thickness,
                s.ssr_roughness,
            ],
            scatter: [s.sss_strength, s.sss_radius, 0.0, focus],
            lens: [s.dof_aperture, s.dof_max_blur, s.anamorphic, s.lens_flare],
            // `rays.z` niesie `ssao_reject_fadeoff`. Parametr z jednej
            // grupy (`rays`) trafia do drugiej (`ssao`) — celowo: `rays`
            // ma wolne `z`, a dopisanie nowego `vec4` do uniformu
            // przesunęłoby wszystkie późniejsze pola i wymusiłoby zmianę
            // w WGSL. Jeden przeskok między grupami jest tańszy niż
            // przesunięcie 9 bindingów.
            rays: [s.god_rays, s.god_ray_density, s.ssao_reject_fadeoff, 0.0],
            depth: [near, far, 0.0, 0.0],
            grade2: [s.split_tone, 0.0, s.split_tone, 0.0],
            ssao: [
                s.ssao_strength,
                s.ssao_radius,
                s.ssao_bias,
                s.ssao_intensity,
            ],
            // Projekcja: SSAO odtwarza z tego pozycję w metrach.
            proj: [tan_half_fov, aspect, 0.0, 0.0],
        }
    }
}

const _: () = assert!(std::mem::size_of::<PostParams>() == 208);

/// Rozmiar bufora uniformu w bajtach — jedna stała dla wszystkich passów.
///
/// Trzymamy ją obok struktury, bo `create_buffer` potrzebuje liczby,
/// a struktury nie da się zmierzyć w wyrażeniu `const`. Asercja
/// powyżej pilnuje, żeby obie wartości nie rozjechały się.
const POST_PARAMS_SIZE: usize = 208;

/// Ustawienia wyglądu, które gra może zmieniać w locie.
#[derive(Debug, Clone, Copy)]
pub struct PostSettings {
    pub exposure: f32,
    pub bloom: f32,
    pub bloom_threshold: f32,
    pub vignette: f32,
    pub chromatic: f32,
    pub grain: f32,
    pub saturation: f32,
    pub contrast: f32,
    /// Siła antyaliasingu krawędzi 0..1.
    pub antialias: f32,

    // --- nowe grupy efektów ---------------------------------------------
    /// Siła konturów ekranowych 0..1 (`0` = wyłączone).
    ///
    /// Uwaga: to NIE jest kontur odwróconej powłoki. Zobacz `fs_composite`
    /// w `post.wgsl` — kontur liczymy z różnic głębokości i normalnych
    /// i przygaszamy go w jasnych miejscach.
    pub outline_strength: f32,
    /// Grubość konturu w pikselach (przed pomnożeniem przez rozmiar kadru).
    pub outline_width: f32,
    /// Próg wykrywania krawędzi — niższy = ciańszy kontur.
    pub outline_threshold: f32,
    /// Jak mocno kontur znika w jasnych miejscach 0..1.
    pub outline_fade: f32,

    /// Siła odbić ekranowych (SSR) 0..1.
    pub ssr_strength: f32,
    /// Maksymalna chropowatość, przy której odbicie jeszcze liczymy.
    /// Powyżej powierzchnia jest zbyt matowa, żeby coś odbić.
    pub ssr_roughness: f32,
    /// Grubość tolerancji głębokości dla trafienia promienia.
    pub ssr_thickness: f32,

    /// Siła podpowierzchniowego rozpraszania w post-processingu 0..1.
    pub sss_strength: f32,
    /// Promień SSS w pikselach.
    pub sss_radius: f32,

    /// Przysłona (mocne rozmycie tła). `0` = cały kadr ostry.
    pub dof_aperture: f32,
    /// Ogniskowa odległość w jednostkach świata.
    ///
    /// Wartość `0.0` oznacza „auto": renderer ustawia ją na odległość
    /// od środka kadru, co daje symulację aparatu ustawionego na
    /// to, na co gracz patrzy.
    pub dof_focus: f32,
    /// Maksymalny promień rozmycia w pikselach.
    pub dof_max_blur: f32,

    /// Siła anamorficznej (poziomej) poświaty 0..1.
    pub anamorphic: f32,
    /// Siła flary obiektywu 0..1.
    pub lens_flare: f32,
    /// Siła promieni słonecznych (god rays) 0..1.
    pub god_rays: f32,
    /// Gęstość smug god rays: dzielnik kroku marszu (`krok / gęstość`).
    ///
    /// UWAGA: to NIE jest zasięg smugi w kadrze, tylko jak gęsto są
    /// próbki wzdłuż promienia. Większa wartość = krótszy krok = krótsze,
    /// częstsze smugi. Shader liczy `N / gęstość` próbek na piksel.
    pub god_ray_density: f32,
    /// Siła podziału tonów (chłodne cienie / ciepłe światła) 0..1.
    pub split_tone: f32,

    // --- SSAO -----------------------------------------------------------
    /// Siła cieniowania ekranowego 0..1 (`0` = wyłączone).
    ///
    /// SSAO przyciemnia miejsca, w których otoczenie zasłania samo
    /// siebie: narożniki ścian, styki obiektów z podłogą, wnęki. To
    /// najtańszy sposób na „doklejenie" obiektów do świata — bez niego
    /// bryły wiszą w powietrzu i brzmią jak papier.
    pub ssao_strength: f32,
    /// Promień próbkowania w jednostkach świata.
    ///
    /// Za mały = czarne plamy przy kontaktach, za duży = szare smugi
    /// rozlewające się po dużych powierzchniach. 0.6 m odpowiada
    /// człowiekowi stojącemu w drzwiach.
    pub ssao_radius: f32,
    /// Przesunięcie próbki wzdłuż normalnej, w jednostkach świata.
    ///
    /// Chroni przed samooczytywaniem (*surface acne*), które bez tego
    /// daje czarne kropki na płaskich powierzchniach.
    pub ssao_bias: f32,
    /// Potęga zaciennienia — jak głęboko cienie w klach.
    ///
    /// `1.0` = liniowe cieniowanie, `>1` = bardziej dramatyczne narożniki.
    pub ssao_intensity: f32,
    /// Stromość odrzucania próbek w teście par (MSAO).
    ///
    /// ## Skąd to się wzięło
    ///
    /// Test „kulowy" (`dot(V, N) > 0 i |V| < radius`) traktuje próbkę
    /// leżącą na **tej samej powierzchni** co zasłaniającą, bo formalnie
    /// mieści się w kuli. W praktyce daje to ciemne plamy tam, gdzie
    /// powierzchnia zakrzywia się w stronę kamery.
    ///
    /// MSAO (WickedEngine, `msaoCS.hlsl`, pochodna MiniEngine) rozwiązuje
    /// to testem **par**: bierze próbki po obu stronach piksela
    /// (`+offset` i `-offset`) i porównuje je ze sobą. Obie mierzą to
    /// samo przenikanie w głąb kuli, więc ich różnica kasuje artefakt
    /// powierzchni, a zostawia prawdziwe przesłony.
    ///
    /// To `xRejectFadeoff` z MSAO. Wyższa = ostrzejsze odrzucanie, ale
    /// powyżej ~250 próbki na płaskiej powierzchni zaczynają
    /// „przebijać" i AO znika. Zakres użyteczny: 50..200.
    pub ssao_reject_fadeoff: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            // Ekspozycja 1.0: neutralny punkt odniesienia AgX. Wyższa
            // wartość to już decyzja artystyczna i należy do demo,
            // nie do domyślnego ustawienia silnika.
            exposure: 1.0,
            // Bloom 0.32 → 0.18. Domyślne 0.32 przy progu 1.15 dawało
            // poświatę na KAŻDEJ jasnej powierzchni powyżej progu, nie
            // tylko na źródłach — tło nieba i słońce obracały się w
            // białą plamę. 0.18 zostawia poświatę tam, gdzie jest
            // źródło, a resztę powierzchni nietkniętą.
            bloom: 0.18,
            // Próg wysoko ponad 1.0: poświatę dostaje tylko rdzeń
            // reaktora, wizjer i tarcza słońca, a nie każda jasna ściana.
            bloom_threshold: 1.15,
            // 0.30 → 0.18. Winieta ma delikatnie zamykać kadr; przy 0.30
            // narożnik był ciemniejszy o ~30% i wyglądał jak defekt
            // kadru, a nie jak optyka.
            vignette: 0.18,
            // Bardzo słaba aberracja: łamie idealną gładkość krawędzi,
            // ale jest niewidoczna gołym okiem w ruchu.
            chromatic: 0.0012,
            // Ziarno 0.014 → 0.007. Jeszcze wystarcza na maskowanie
            // bandingu w gradiencie nieba, a nie jest widoczną teksturą.
            grain: 0.007,
            // Nasycenie 1.10 → 1.04 i kontrast 1.05 → 1.03. Silnik ma
            // dawać wierny obraz; wyraźna stylizacja to decyzja demo.
            saturation: 1.04,
            contrast: 1.03,
            // Antyaliasing to NIE jest efekt, tylko redukcja aliasingu.
            // Obniżanie go dodawałoby schodki zamiast je usuwać, więc
            // zostaje 0.65.
            antialias: 0.65,

            // --- kontury ---
            // 0.55 → 0.02. Kontur praktycznie wyłączony — zostaje
            // jako najcieńsza akcentacja sylwetki, bo przy 0.02 jego
            // wkład w obraz jest mniejszy niż szum ziarna.
            outline_strength: 0.02,
            // 1.2 → 1.0 px. Przy 1.0 kontur bywa niewidoczny na
            // krawędziach normalnych, przy 2.0 robi się gruby
            // i „obłotowy".
            outline_width: 1.0,
            // 0.010: niżej kontur łapie szum z głębokości, wyżej
            // gubi krawędzie obiektów stojących daleko.
            outline_threshold: 0.010,
            // 0.75 = kontur prawie znika na słońcu, jest w cieniu.
            outline_fade: 0.75,

            // --- SSR ---
            // 0.65 → 0.11. Odbicie ledwie widoczne: odbijają się
            // tylko naprawdyle gładkie powierzchnie (szyby, woda przy
            // krawędzi), a metal i mokry beton wyglądają jak suche.
            ssr_strength: 0.11,
            // 0.55: powyżej tej chropowatości odbicie jest szumem.
            ssr_roughness: 0.55,
            // 1.5 m tolerancji — przy mniejszej odbicie „przepływa"
            // przez cienkie listwy, przy większej łapie sąsiednie ściany.
            ssr_thickness: 1.5,

            // --- SSS ---
            // 0.5 → 0.30: widoczna ciepła poświata na krawędziach skóry,
            // bez efektu „podświetlonej zielonej główki".
            sss_strength: 0.30,
            sss_radius: 2.5,

            // --- DoF ---
            // Apertura 1.0 → 0.7: miękkie rozmycie tła w rozmowie
            // lub ataku. Więcej niż 1.0 zamienia obraz w plamę
            // i utrudnia odczytanie przeciwnika w tle.
            dof_aperture: 0.7,
            // 0.0 = autofocus (renderer policzy z kadru).
            dof_focus: 0.0,
            // 6.0 → 0.10 px: DoF praktycznie wyłączony. 0.10 px to
            // mniej niż połowa texela, więc kadr jest praktycznie cały
            // ostry. Zostawiamy małą wartość zamiast 0.0, bo przy zerze
            // `1/max(d, 0.001)` w shaderze dawałoby nieskończony blur
            // na pikselach leżących w płaszczyźnie ogniskowania.
            dof_max_blur: 0.10,

            // --- obiektyw ---
            // 0.35 → 0.18: poświata anamorficzna zaznaczona tylko przy
            // najjaśniejszych źródłach. Powyżej 0.6 każda jasna plama
            // dostaje poziomą kreskę i obraz wygląda „chory".
            anamorphic: 0.18,
            // 0.25 → 0.15: ghosty mają być ledwie zauważalne.
            lens_flare: 0.15,
            // 0.4 → 0.25: smugi widoczne, gdy patrzymy w stronę słońca,
            // bez ciągnięcia się przez środek kadru.
            god_rays: 0.25,
            // 1.0 = krótki, gęsty wzór próbek. Większa wartość daje
            // DŁUŻSZE, rzadsze smugi (krok maleje), mniejsza — krótsze.
            god_ray_density: 1.0,
            // 0.5 → 0.35: chłodne cienie i ciepłe światła, bez przesady.
            split_tone: 0.35,

            // --- SSAO ---
            // 0.55 → 0.21: zacienienie ledwie widoczne. Przy tej
            // sile AO wciąż dokleja obiekty do podłoża, ale nie
            // przyciemnia całej bryły — zostaje cień styku, nie plama.
            ssao_strength: 0.21,
            // 0.35 → 0.12 m: promień zbliżony do 12 cm. Zacienienie
            // sięga wtedy tylko najbliższego otoczenia piksela, a nie
            // całego obrysu obiektu. Dla postaci stojącej na ziemi
            // to dokładnie odległość, w której stopa styka się z
            // podłożem; dla podłogi — znikające plamy wyglądające
            // jak brud.
            ssao_radius: 0.12,
            // 0.02 m marginesu grubości kuli: tyle, żeby próbka
            // nie czytała tej samej powierzchni, którą cieniuje.
            ssao_bias: 0.02,
            // 1.4 → 1.2: łagodniejsza krzywa potęgi — słabe zaciennienia
            // znikają, mocne w szczelinach zostają. Powyżej 2.0 AO
            // zaczyna wyglądać jak brud na obiektywie.
            ssao_intensity: 1.2,
            // 120 = odrzucanie w teście par (MSAO). Wartość z zakresu
            // użytecznego: zbyt mała zostawia plamy na powierzchniach
            // zakrzywionych ku kamerze, zbyt duża kasuje AO przy
            // stykach obiektów. WickedEngine używa tu ~120.
            ssao_reject_fadeoff: 120.0,
        }
    }
}

/// Wszystkie zasoby post-processingu.
pub struct PostFx {
    format: wgpu::TextureFormat,
    /// ZAWSZE 1. HDR jest próbkowane jako zwykła `texture_2d`, a taka
    /// próbka musi mieć `sample_count = 1`; przy MSAA wgpu odrzuci
    /// bind group. Krawędzie łagodzi shader (`fs_composite`), a nie
    /// multisampling — dlatego `uran-tanks` ustawia `.samples(1)`.
    samples: u32,

    /// Cel HDR sceny — tu trafiają bryły i niebo, dopóki nie przejdzie
    /// przez tonemapping.
    hdr: Option<wgpu::TextureView>,
    /// G-Bufer: normalna w przestrzeni oka (xyz) + chropowatość (w).
    ///
    /// Osobny cel renderowania, bo `wgpu` nie pozwala czytać tekstury,
    /// która w tym samym passie jest jego celem — a efekty ekranowe
    /// (SSR, kontury, SSS, DoF) MUSZĄ czytać to, co właśnie narysowała
    /// scena. Stąd zamiast rekonstrukcji z koloru mamy prawdziwe dane.
    gbuffer: Option<wgpu::TextureView>,
    /// Kopia głębokości z możliwością próbkowania.
    ///
    /// Bufor głębokości w `scene.rs` ma wyłącznie `RENDER_ATTACHMENT`:
    /// wgpu nie pozwala dołączyć go jako źródła w tym samym passie.
    /// Dlatego po narysowaniu sceny KOPIUJEMY go do tekstury, którą
    /// już można czytać. Koszt to jeden `copy_texture_to_texture`
    /// na klatkę.
    ///
    /// Trzymamy `Texture`, a nie `TextureView`, bo kopia potrzebuje
    /// obiektu tekstury, nie samego widoku.
    depth_copy: Option<wgpu::Texture>,
    /// Widok `depth_copy` — tworzony raz w `ensure_size`, bo `TextureView`
    /// nie implementuje `Clone` w wgpu 0.19.
    depth_view_cache: Option<wgpu::TextureView>,
    /// Dwa cele pośrednie bloom, oba w połowie rozmiaru kadru.
    bloom_a: Option<wgpu::TextureView>,
    bloom_b: Option<wgpu::TextureView>,
    /// Cele dla passów ekranowych, oba w połowie rozmiaru kadru.
    ///
    /// DWA osobne, bo każdy pass nadpisałby wynik poprzedniego, a
    /// kompozycja czyta je równocześnie (odbicie + poświat słoneczna).
    /// Trzymanie ich w połowie rozmiaru jest tu prawie darmowe: oba
    /// efekty liczą różnice sąsiednich pikseli, więc wynik i tak jest
    /// rozmyty przez downsampling, a pełna rozdzielczość kosztowałaby
    /// czterokrotnie więcej próbek.
    ssr_out: Option<wgpu::TextureView>,
    rays_out: Option<wgpu::TextureView>,
    /// Wynik SSAO: jeden kanał zacienienia (R8), w POŁOWIE rozmiaru
    /// kadru.
    ///
    /// AO to zjawisko niskoczęstotliwości: narożnik ma kilka pikseli
    /// szerokości, a detale map normalnych są dla niego szumem. Połowa
    /// rozdzielczości daje 4× mniej próbek i przy rozmyciu w dół
    /// wygląda identycznie.
    ao_out: Option<wgpu::TextureView>,
    /// Zapis z SSAO przed rozmyciem — osobna tekstura, bo `ao_out`
    /// jest celem tego samego passu i nie może być jednocześnie źródłem.
    ao_raw: Option<wgpu::TextureView>,
    sized: (u32, u32),

    params_buffer: wgpu::Buffer,
    /// Trzy bufory parametrów: bright, blur poziomy, blur pionowy.
    blur_buffers: [wgpu::Buffer; 3],

    layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    bright_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    /// Odbicia ekranowe (SSR) — patrz `fs_ssr` w `post.wgsl`.
    ssr_pipeline: wgpu::RenderPipeline,
    /// Promienie słoneczne — patrz `fs_godray`.
    godray_pipeline: wgpu::RenderPipeline,
    /// SSAO — patrz `fs_ssao`.
    ssao_pipeline: wgpu::RenderPipeline,
    /// Rozmycie SSAO w dół (4×4 box) — patrz `fs_ao_blur`.
    ao_blur_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Bind grupy zależne od tekstur — tworzone w `ensure_size`.
    composite_bg: Option<wgpu::BindGroup>,
    /// 0: hdr -> bright, 1: bloom_a -> blur H, 2: bloom_b -> blur V
    bloom_bgs: [Option<wgpu::BindGroup>; 3],
    /// 0: SSR, 1: god rays — oba czytają G-Bufer i głębokość sceny.
    effect_bgs: [Option<wgpu::BindGroup>; 2],
    /// 0: SSAO (gbuffer+depth -> ao_raw), 1: rozmycie AO (ao_raw -> ao_out).
    ao_bgs: [Option<wgpu::BindGroup>; 2],
    settings: PostSettings,
}

impl PostFx {
    /// Tworzy potoki i bufory. `format` musi być formatem powierzchni.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let params_buffer = uniform(device, "Uran Post Params", POST_PARAMS_SIZE);
        let blur_buffers = [
            uniform(device, "Uran Blur Bright", POST_PARAMS_SIZE),
            uniform(device, "Uran Blur H", POST_PARAMS_SIZE),
            uniform(device, "Uran Blur V", POST_PARAMS_SIZE),
        ];

        // JEDEN layout obsługuje wszystkie cztery passy: bright/blur/
        // composite mają ten sam zestaw bindingów, a nieużywane pomija
        // WGSL. Dzięki temu mamy jeden bind group zamiast trzech layoutów.
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran Post BGL"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(4),
                // Głębokość jest formatem niefiltrowalnym: `Depth` + brak
                // `filterable` daje `texture_depth_2d` w WGSL. Dla
                // `textureSample` potrzebny byłby `sampler_comparison`,
                // ale my czytamy ją przez `textureLoad` (patrz
                // `view_depth` w `post.wgsl`), więc zwykły sampler
                // wystarczy i unika odrzucenia layoutu.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                texture_entry(6),
                texture_entry(7),
                // Mapa SSAO (R8Unorm). Osobny binding, bo `ssr_tex`
                // i `rays_tex` to tekstury RGBA16Float, a AO ma inny
                // format — wgpu nie pozwala zadeklarować jednego slotu
                // dla dwóch formatów.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Uran Post Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran Post Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });

        let composite_pipeline =
            pipeline(device, format, samples, &layout, &module, "fs_composite");
        // cele pośrednie bloom są 1:1 (bez MSAA) — rozmycie i tak gubi
        // szczegóły, a 4x więcej próbek kosztowałoby bez efektu
        let bright_pipeline = pipeline(device, HDR_FORMAT, 1, &layout, &module, "fs_bright");
        let blur_pipeline = pipeline(device, HDR_FORMAT, 1, &layout, &module, "fs_blur");
        // Pasy ekranowe (SSR, god rays) liczą się w POŁOWIE rozmiaru
        // kadru: to dobór `ensure_size`. DoF i kontury działają w
        // kompozycji, w pełnej rozdzielczości.
        let ssr_pipeline = pipeline(device, HDR_FORMAT, 1, &layout, &module, "fs_ssr");
        let godray_pipeline = pipeline(device, HDR_FORMAT, 1, &layout, &module, "fs_godray");
        // SSAO liczymy w POŁOWIE kadru (jak SSR i god rays) i czyta
        // tylko G-Bufer + głębokość. Ten sam celowy format co reszta
        // pośrednich, bo `ao_raw` musi dać się czytać jako `texture_2d`.
        let ssao_pipeline = pipeline(device, AO_FORMAT, 1, &layout, &module, "fs_ssao");
        let ao_blur_pipeline = pipeline(device, AO_FORMAT, 1, &layout, &module, "fs_ao_blur");

        Self {
            format,
            samples,
            hdr: None,
            gbuffer: None,
            depth_copy: None,
            depth_view_cache: None,
            bloom_a: None,
            bloom_b: None,
            ssr_out: None,
            rays_out: None,
            ao_out: None,
            ao_raw: None,
            sized: (0, 0),
            params_buffer,
            blur_buffers,
            layout,
            composite_pipeline,
            bright_pipeline,
            blur_pipeline,
            ssr_pipeline,
            godray_pipeline,
            ssao_pipeline,
            ao_blur_pipeline,
            sampler,
            composite_bg: None,
            bloom_bgs: [None, None, None],
            effect_bgs: [None, None],
            ao_bgs: [None, None],
            settings: PostSettings::default(),
        }
    }

    /// Ustawienia wyglądu (do dostrojenia z gry).
    pub fn settings_mut(&mut self) -> &mut PostSettings {
        &mut self.settings
    }

    /// Format celu HDR: 16-bitowy float.
    ///
    /// `Rgba8Unorm` obciąłby wszystko powyżej 1.0, czyli dokładnie
    /// poświaty i tarczę słońca — a o nie chodzi w bloomie.
    pub fn hdr_format(&self) -> wgpu::TextureFormat {
        HDR_FORMAT
    }

    /// Cel HDR — do niego rysuje scena.
    pub fn hdr_view(&self) -> &wgpu::TextureView {
        self.hdr.as_ref().expect("ensure_size przed hdr_view")
    }

    /// G-Bufer: normalna w przestrzeni oka + chropowatość.
    ///
    /// Drugi cel kolorowy passu sceny. Post-processing czyta go w
    /// osobnych passach (po zakończeniu renderowania sceny), więc
    /// wgpu nie widzi konfliktu „attachment i źródło w jednym passie".
    pub fn gbuffer_view(&self) -> &wgpu::TextureView {
        self.gbuffer
            .as_ref()
            .expect("ensure_size przed gbuffer_view")
    }

    /// Kopia głębokości gotowa do próbkowania (patrz pole `depth_copy`).
    pub fn depth_view(&self) -> &wgpu::TextureView {
        // `create_view` na żądanie zamiast trzymanego pola: `TextureView`
        // nie implementuje `Clone` w wgpu 0.19, a bind grupy potrzebuje
        // referencji, która żyje dłużej niż wywołanie. Trzymanie jednego
        // widoku w polu wymagałoby dodatkowej opcji i osobnej obsługi
        // przy `ensure_size`.
        self.depth_view_cache
            .as_ref()
            .expect("ensure_size przed depth_view")
    }

    /// Zapisuje encję kopiującą głębokość sceny do tekstury dla postfx.
    ///
    /// Wywoływane z `scene.rs` PO zakończeniu passu głównego: w tym
    /// samym passie nie wolno czytać tekstury, która jest jego celem.
    pub fn copy_depth(&self, enc: &mut wgpu::CommandEncoder, src: &wgpu::Texture) {
        let Some(dst) = self.depth_copy.as_ref() else {
            return;
        };
        enc.copy_texture_to_texture(
            wgpu::ImageCopyTexture {
                texture: src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyTexture {
                texture: dst,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.sized.0.max(1),
                height: self.sized.1.max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    /// Tekstura z wynikiem SSR (czytana przez kompozycję).
    pub fn ssr_view(&self) -> &wgpu::TextureView {
        self.ssr_out.as_ref().expect("ensure_size przed ssr_view")
    }

    /// Tekstura z wynikiem god rays (czytana przez kompozycję).
    pub fn rays_view(&self) -> &wgpu::TextureView {
        self.rays_out.as_ref().expect("ensure_size przed rays_view")
    }

    /// Wymusza odtworzenie tekstur przy najbliższym `composite`
    /// (po zmianie rozmiaru okna).
    pub fn invalidate(&mut self) {
        self.sized = (0, 0);
    }
}

impl PostFx {
    /// Tworzy tekstury i bind grupy dla aktualnego rozmiaru okna.
    pub fn ensure_size(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.sized == (w, h) {
            return;
        }
        self.sized = (w, h);

        self.hdr = Some(color_view(
            device,
            w,
            h,
            "Uran Post HDR",
            HDR_FORMAT,
            self.samples,
        ));
        // G-Bufer: ten sam format co HDR, bo normalna w 8 bitach daje
        // widoczne „schodki" na konturach (krawędź normalnej to różnica
        // rzędu 0.02, a 8 bitów na kanał daje krok 0.004 — na granicy
        // artefaktu).
        self.gbuffer = Some(color_view(
            device,
            w,
            h,
            "Uran Post GBuffer",
            HDR_FORMAT,
            self.samples,
        ));
        // Kopia głębokości. Format MUSI być identyczny z buforem
        // głębokości w `scene.rs` — `copy_texture_to_texture` wymaga
        // zgodności formatów, a przy `Depth24Plus` vs `Depth32Float`
        // wgpu odrzuci kopię.
        self.depth_copy = Some(depth_texture(device, w, h, "Uran Post Depth"));
        self.depth_view_cache = Some(
            self.depth_copy
                .as_ref()
                .expect("właśnie utworzone")
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        let (bw, bh) = ((w / 2).max(1), (h / 2).max(1));
        self.bloom_a = Some(color_view(device, bw, bh, "Uran Bloom A", HDR_FORMAT, 1));
        self.bloom_b = Some(color_view(device, bw, bh, "Uran Bloom B", HDR_FORMAT, 1));
        // Efekty ekranowe w POŁOWIE rozmiaru: oba liczą różnice
        // sąsiednich pikseli (promień SSR, radialne smugi), więc wynik
        // i tak jest rozmyty przez downsampling.
        let ssr = Some(color_view(device, bw, bh, "Uran SSR Out", HDR_FORMAT, 1));
        let rays = Some(color_view(device, bw, bh, "Uran Rays Out", HDR_FORMAT, 1));
        self.ssr_out = ssr;
        self.rays_out = rays;
        // SSAO: dwa bufory w POŁOWIE kadru. `ao_raw` to wynik surowy
        // (wysoki szum), `ao_out` — po rozmyciu, który czyta kompozycja.
        // Osobne, bo pass nie może czytać swojego celu.
        self.ao_raw = Some(color_view(device, bw, bh, "Uran AO Raw", AO_FORMAT, 1));
        self.ao_out = Some(color_view(device, bw, bh, "Uran AO Out", AO_FORMAT, 1));

        let hdr = self.hdr.as_ref().expect("właśnie utworzone");
        let ba = self.bloom_a.as_ref().expect("właśnie utworzone");
        let bb = self.bloom_b.as_ref().expect("właśnie utworzone");
        let gbuf = self.gbuffer.as_ref().expect("właśnie utworzone");
        let depth = self.depth_view_cache.as_ref().expect("właśnie utworzone");
        let ssr_tex = self.ssr_out.as_ref().expect("właśnie utworzone");
        let rays_tex = self.rays_out.as_ref().expect("właśnie utworzone");
        let ao_raw = self.ao_raw.as_ref().expect("właśnie utworzone");
        let ao_out = self.ao_out.as_ref().expect("właśnie utworzone");
        // w bright/blur binding 2 to samo źródło — nieużywany przez WGSL.
        // Pasy te nie czytają `ssr_tex` ani `rays_tex`, więc dostają je
        // tylko po to, by wpis w layout zgadzał się z liczbą wpisów.
        //
        // Slot 8 to AO. Passy, które go nie czytają (bloom, SSR, god
        // rays), dostają `ao_out` — jest neutralny i nigdy nie jest
        // celem żadnego z tych passów.
        self.bloom_bgs[0] = Some(self.bind(
            device,
            &self.blur_buffers[0],
            hdr,
            hdr,
            gbuf,
            depth,
            ssr_tex,
            rays_tex,
            ao_out,
        ));
        self.bloom_bgs[1] = Some(self.bind(
            device,
            &self.blur_buffers[1],
            ba,
            ba,
            gbuf,
            depth,
            ssr_tex,
            rays_tex,
            ao_out,
        ));
        self.bloom_bgs[2] = Some(self.bind(
            device,
            &self.blur_buffers[2],
            bb,
            bb,
            gbuf,
            depth,
            ssr_tex,
            rays_tex,
            ao_out,
        ));
        // Kompozycja czyta naraz: HDR, bloom, odbicia, smugi i AO.
        self.composite_bg = Some(self.bind(
            device,
            &self.params_buffer,
            hdr,
            ba,
            gbuf,
            depth,
            ssr_tex,
            rays_tex,
            ao_out,
        ));
        // Każdy pass ekranowy renderuje DO swojego bufora, więc NIE
        // wiążemy `ssr_out`/`rays_out` w jego własnej grupie: wgpu
        // odrzuca zasób użyty jednocześnie jako attachment i źródło.
        // Sloty, których dany shader nie czyta, dostają `hdr` —
        // jest neutralny i nigdy nie jest celem żadnego z tych passów.
        self.effect_bgs[0] = Some(self.bind(
            device,
            &self.params_buffer,
            hdr,
            hdr,
            gbuf,
            depth,
            hdr,
            hdr,
            ao_out,
        ));
        self.effect_bgs[1] = Some(self.bind(
            device,
            &self.params_buffer,
            hdr,
            hdr,
            gbuf,
            depth,
            hdr,
            hdr,
            ao_out,
        ));
        // SSAO czyta gbuffer + depth i pisze do `ao_raw`; więc NIE
        // wiąże `ao_raw` w jego własnej grupie. `fs_ssao` nie czyta
        // slotu 8 w ogóle, więc dostaje `ao_out` tylko dla zgodności
        // liczby wpisów z layoutem.
        self.ao_bgs[0] = Some(self.bind(
            device,
            &self.params_buffer,
            hdr,
            hdr,
            gbuf,
            depth,
            hdr,
            hdr,
            ao_out,
        ));
        // Rozmycie AO: `fs_ao_blur` czyta slot 8, więc to MUSI być
        // `ao_raw` — celem jest `ao_out`, a wgpu odrzuca zasób użyty
        // w jednym passie jako źródło i jako color target naraz.
        self.ao_bgs[1] = Some(self.bind(
            device,
            &self.params_buffer,
            ao_raw,
            ao_raw,
            gbuf,
            depth,
            hdr,
            hdr,
            ao_raw,
        ));
    }

    /// Bind grupa: uniform + cztery tekstury + sampler.
    ///
    /// Rozdzielenie `aux` (bloom) i `effects` jest konieczne tylko dla
    /// kompozycji, która czyta jedno i drugie naraz. Pozostałe passy
    /// dostają `effects = aux`, bo nie używają tego bindingu.
    #[allow(clippy::too_many_arguments)]
    fn bind(
        &self,
        device: &wgpu::Device,
        params: &wgpu::Buffer,
        src: &wgpu::TextureView,
        aux: &wgpu::TextureView,
        gbuf: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        ssr: &wgpu::TextureView,
        rays: &wgpu::TextureView,
        ao: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran Post BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(aux),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(gbuf),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                // Dwa osobne cele efektów. Nie mogą się dzielić buforem,
                // bo kompozycja czyta je równocześnie.
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(ssr),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(rays),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(ao),
                },
            ],
        })
    }

    /// Wpisuje uniformy na bieżącą klatkę.
    ///
    /// Jeden `write_buffer` na pass — niezależnie od liczby obiektów
    /// w scenie.
    ///
    /// `sun_uv` to pozycja słońca w UV (albo `None`, gdy słońce jest poza
    /// kadrem), a `focus` to ogniskowa odległość DoF. Oba wylicza
    /// `Renderer3d`, bo wymagają wiedzy o kamerze, której tu nie mamy.
    pub fn upload_params(
        &self,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        time: f32,
        sun_uv: Option<(f32, f32)>,
        near: f32,
        far: f32,
        tan_half_fov: f32,
        aspect: f32,
    ) {
        let s = &self.settings;
        // Słońce: `z > 0` to przełącznik widoczności w shaderze. Poza
        // kadrem ustawiamy 0, żeby flara i promienie się wyłączyły —
        // inaczej pojawiałyby się z krawędzi obrazu.
        let sun = match sun_uv {
            Some((x, y)) => [x, y, 1.0, 0.0],
            None => [0.5, 0.5, 0.0, 0.0],
        };
        // Autofocus: `0.0` w ustawieniach znaczy „ustaw na to, co
        // gracz patrzy". Wartość 1.0 m jest bezpiecznym minimum —
        // przy mniejszym promień DoF rósłby do nieskończoności
        // przez `1/max(d, 0.001)` w shaderze.
        let focus = if s.dof_focus > 0.0 { s.dof_focus } else { 1.0 };

        let main = PostParams::from_settings(
            s,
            w as f32,
            h as f32,
            time,
            sun,
            near,
            far,
            focus,
            tan_half_fov,
            aspect,
        );
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&main));

        // bright: próg w `fx.x`, kierunek w `grade.x` jest tu nieistotny.
        // Pasy bloom nie czytają nowych grup, więc wystarczy wypełnić
        // je zerami — jawnie, żeby nie było pól „niezainicjalizowanych".
        let bright = PostParams {
            grade: [0.0; 4],
            fx: [s.bloom_threshold, 0.0, 1.0, 1.0],
            screen: [w as f32, h as f32, 0.0, 0.0],
            sun: [0.0; 4],
            outline: [0.0; 4],
            ssr: [0.0; 4],
            scatter: [0.0; 4],
            lens: [0.0; 4],
            rays: [0.0; 4],
            depth: [near, far, 0.0, 0.0],
            grade2: [0.0; 4],
            ssao: [0.0; 4],
            proj: [0.0; 4],
        };
        queue.write_buffer(&self.blur_buffers[0], 0, bytemuck::bytes_of(&bright));

        // blur: `grade.x` = kierunek (1 = poziomo, 0 = pionowo),
        // `screen.xy` = rozmiar ŹRÓDŁA, bo krok kernela jest w UV.
        // Ważne: to rozmiar bufora bloom (połowa kadru), a nie kadru —
        // inaczej kernel byłby dwa razy za szeroki.
        let mut blur_h = bright;
        blur_h.grade[0] = 1.0;
        blur_h.screen = [(w / 2).max(1) as f32, (h / 2).max(1) as f32, 0.0, 0.0];
        queue.write_buffer(&self.blur_buffers[1], 0, bytemuck::bytes_of(&blur_h));
        let mut blur_v = blur_h;
        blur_v.grade[0] = 0.0;
        queue.write_buffer(&self.blur_buffers[2], 0, bytemuck::bytes_of(&blur_v));
    }
}

/// Bufor uniformu o zadanym rozmiarze w bajtach.
fn uniform(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl PostFx {
    /// Wykonuje pełny łańcuch post-processingu i kompozycję na powierzchnię.
    ///
    /// Kolejność:
    /// 1. god rays (HDR → `rays_out`)
    /// 2. SSR (HDR + G-Bufer + głębokość → `ssr_out`)
    /// 3. bloom: bright → blur H → blur V
    /// 4. kompozycja na powierzchnię
    ///
    /// Oba passy ekranowe mają WŁASNE cele, więc ich wyniki nie
    /// nadpisują się nawzajem, a kompozycja czyta je równocześnie.
    ///
    /// Wszystkie passy idą w TYM SAMYM encoderze, więc GPU nie czeka
    /// na osobne synchronizacje między nimi.
    pub fn composite(&self, enc: &mut wgpu::CommandEncoder, target: &Scene3dTarget<'_>) {
        let ba = self.bloom_a.as_ref().expect("ensure_size");
        let bb = self.bloom_b.as_ref().expect("ensure_size");
        let ssr = self.ssr_out.as_ref().expect("ensure_size");
        let rays = self.rays_out.as_ref().expect("ensure_size");
        let ao_raw = self.ao_raw.as_ref().expect("ensure_size");
        let ao = self.ao_out.as_ref().expect("ensure_size");

        // --- SSAO: gbuffer + depth -> ao_raw -> ao_out
        //
        // PRZED kompozycją, bo to ona czyta AO. Kolejność względem
        // SSR i god rays nie ma znaczenia — te zapisują w swoje
        // własne cele i czytają surowy HDR, a nie wynik AO.
        if let Some(bg) = self.ao_bgs[0].as_ref() {
            self.fullscreen(enc, "Uran SSAO", &self.ssao_pipeline, bg, ao_raw);
        }
        if let Some(bg) = self.ao_bgs[1].as_ref() {
            self.fullscreen(enc, "Uran AO Blur", &self.ao_blur_pipeline, bg, ao);
        }

        // --- god rays: hdr -> rays_out
        if let Some(bg) = self.effect_bgs[1].as_ref() {
            self.fullscreen(enc, "Uran God Rays", &self.godray_pipeline, bg, rays);
        }

        // --- SSR: hdr + gbuffer + depth -> ssr_out
        if let Some(bg) = self.effect_bgs[0].as_ref() {
            self.fullscreen(enc, "Uran SSR", &self.ssr_pipeline, bg, ssr);
        }

        // --- bright-pass: hdr -> bloom_a
        if let Some(bg) = self.bloom_bgs[0].as_ref() {
            self.fullscreen(enc, "Uran Bright", &self.bright_pipeline, bg, ba);
        }
        // --- rozmycie poziome: bloom_a -> bloom_b
        if let Some(bg) = self.bloom_bgs[1].as_ref() {
            self.fullscreen(enc, "Uran Blur H", &self.blur_pipeline, bg, bb);
        }
        // --- rozmycie pionowe: bloom_b -> bloom_a
        if let Some(bg) = self.bloom_bgs[2].as_ref() {
            self.fullscreen(enc, "Uran Blur V", &self.blur_pipeline, bg, ba);
        }

        // --- kompozycja na powierzchnię (tu widać efekty)
        let Some(bg) = self.composite_bg.as_ref() else {
            return;
        };
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Uran Composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.color,
                resolve_target: target.resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        p.set_pipeline(&self.composite_pipeline);
        p.set_bind_group(0, bg, &[]);
        p.draw(0..3, 0..1);
    }

    /// Pełnoekranowy pass pośredni (bez depth, jeden trójkąt).
    fn fullscreen(
        &self,
        enc: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
        view: &wgpu::TextureView,
    ) {
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        p.set_pipeline(pipeline);
        p.set_bind_group(0, bind_group, &[]);
        // 3 wierzchołki = pełny prostokąt, bez bufora wierzchołków
        p.draw(0..3, 0..1);
    }
}

/// Pełnoekranowy potok (bloom, kompozycja) — wspólna struktura.
fn pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Uran Post Layout"),
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Uran Post Pipeline"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
            entry_point: "vs_fullscreen",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: entry,
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
    })
}

/// Widok kolorowy do renderowania i próbkowania.
fn color_view(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    label: &str,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            // TEXTURE_BINDING, bo passy bloom i kompozycja je czytają
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// Kopia głębokości gotowa do próbkowania w post-processingu.
///
/// Osobna tekstura, nie ta sama co bufor renderowania: wgpu zabrania
/// użycia zasobu jako attachment i jako źródło w jednym passie.
/// `scene.rs` kopiuje do niej głębokość po zakończeniu renderowania
/// (patrz `PostFx::copy_depth`).
fn depth_texture(device: &wgpu::Device, w: u32, h: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        // Zawsze 1: głębokość multisamplingowa nie da się
        // próbkować jako `texture_depth_2d`. Gry 3D ustawiają
        // `.samples(1)` (patrz `Scene3dTarget`), więc to nie
        // ogranicza — a gdyby ktoś włączył MSAA, walidacja
        // wgpu odrzuci bind grupę z jawnym komunikatem.
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // `Depth32Float` MUSI się zgadzać z formatem bufora
        // głębokości w `scene.rs` — inaczej kopia jest odrzucana.
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Format celów HDR i pośrednich.
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Format mapy SSAO.
///
/// `R8Unorm` wystarcza: AO to jedna wartość 0..1, a 8 bitów daje
/// 256 poziomów cienia — przy rozmyciu w dół nie widać różnicy
/// względem 16-bitów, a cztery razy mniej pamięci.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
