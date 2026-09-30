// Shader 3D — hybrydowy model oświetlenia PBR + NPR w stylu Endfield.
//
// ## Dlaczego hybryda, a nie cel-shading
//
// Klasyczne cel-shading odrzuca fizykę: dwa kolory i koniec. Endfield
// (i cały ten rodzaj „semi-realistic anime") robi coś innego — rozdziela
// materiał na dwie warstwy:
//
//   * **warstwa bazowa** (albedo × diffuse) przechodzi przez miękką
//     rampę, więc postać dostaje czytelne, rysunkowe formy,
//   * **warstwa odbić** (specular / roughness / metallic) zostaje
//     w pełni fizyczna, więc metal nadal odbija otoczenie jak metal.
//
// Efekt: rysunkowa postać w fotorealistycznym otoczeniu, a nie obie
// jednego rodzaju w drugim.
//
// Wszystko liczymy liniowo w HDR; tonemapping i grading są w `post.wgsl`.

struct Scene {
    // Uważaj na wyrównanie w WGSL: samo `vec3<f32>` zajmuje 12 B, ale
    // następna zmienna musi zaczynać się od wielokrotności 16.
    // Dlatego WSZĘDZIE są `vec4`: 400 B łącznie z trzema macierzami.
    view_proj: mat4x4<f32>,      // 64 B
    inv_view_proj: mat4x4<f32>,  // 64 B  (odwrotność powyższej)
    view: mat4x4<f32>,           // 64 B  (świat -> oko)
    eye: vec4<f32>,              // xyz = pozycja oka, w = tan(FOV/2)
    light_dir: vec4<f32>,        // xyz = kierunek światła, w = 0
    light_color: vec4<f32>,      // rgb = kolor światła, a = intensywność
    // rgb = ambient, a = czas świata
    ambient_time: vec4<f32>,
    // --- mapowanie cieni ---
    // Świat -> NDC tekstury cieni (kamera słońca, ortograficzna)
    light_view_proj: mat4x4<f32>,   // 64 B
    // x = bias głębokości, y = odsunięcie wzdłuż normalnej,
    // z = siła cienia, w = promień PCF w texelach
    shadow_params: vec4<f32>,
    // x = rozmiar texela w UV, y = włącznik (0/1)
    shadow_map_info: vec4<f32>,
    // --- ekran i atmosfera ---
    // x,y = rozmiar kadru, z = bliska płaszczyzna, w = daleka
    screen: vec4<f32>,
    // x = siła stylizacji globalnej, y = miękkość progu,
    // z = siła SSS, w = wzmocnienie odbicia otoczenia
    stylize: vec4<f32>,
    // x = gęstość mgły, yzw = kolor mgły
    atmos: vec4<f32>,
    // x = wysokość, na której mgła całkiem zanika (0 = jednolita mgła),
    // yzw = rezerwa na przyszłe parametry pogodowe.
    fog: vec4<f32>,
}

struct Model {
    // mat4x4 + vec4 = 80 B
    model: mat4x4<f32>,
    tint: vec4<f32>,
}

struct Material {
    // x = chropowatość, y = metaliczność, z = mnożnik UV,
    // w = siła normal mappingu
    params: vec4<f32>,
    // x = albedo mapa?, y = normal mapa?, z = ORM mapa?,
    // w = znak odwrócenia V
    flags: vec4<f32>,
    // x = stylizacja, y = anizotropia, z = SSS, w = rim
    surface: vec4<f32>,
    // rgb = emissive (HDR), w = mnożnik diffuse
    tint: vec4<f32>,
    // x = grubość SSS, yzw = kolor SSS
    accent: vec4<f32>,
    // rgb = kolor podstawowy materiału, w = 1 gdy baza pochodzi
    // stąd, 0 gdy bazą jest kolor wierzchołka
    base: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var<storage, read> models: array<Model>;
// --- mapa cieni ---
// `texture_depth_2d` + `sampler_comparison` to para wymagana przez
// `textureSampleCompare`. Zwykły `texture_2d<f32>` nie przyjmie
// porównania, a `sampler` (nie porównawczy) zwróciłby wartość
// zinterpretowaną jako kolor.
@group(0) @binding(2) var shadow_tex: texture_depth_2d;
@group(0) @binding(3) var shadow_samp: sampler_comparison;
@group(1) @binding(0) var albedo_tex: texture_2d<f32>;
@group(1) @binding(1) var normal_tex: texture_2d<f32>;
@group(1) @binding(2) var orm_tex: texture_2d<f32>;
@group(1) @binding(3) var samp: sampler;

// --- szkielet postaci ---
// Macierze kości w grupie 2, a NIE w grupie 0 z pozostałymi zasobami.
// Powód: grupa 0 jest wspólna dla potoku brył i postaci, a siatka
// statyczna nie ma kości — gdyby macierz kości leżała w grupie 0,
// każda bryła świata musiałaby dostać sztuczny bufor 64 kości.
// Dzięki osobnej grupie statyczne DrawCmd w ogóle jej nie dotykają.
@group(2) @binding(0) var<storage, read> bones: array<mat4x4<f32>>;
@group(1) @binding(4) var<uniform> mat: Material;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_pos: vec3<f32>,
    @location(2) color: vec3<f32>,
    // tangent wierzchołkowy — podstawa do normal mappingu
    @location(3) tangent: vec3<f32>,
    @location(4) uv: vec2<f32>,
}

/// Wyjście G-Bufera: kolor HDR (loc 0) + normalna w przestrzeni oka
/// i chropowatość (loc 1).
///
/// Ten drugi cel napędzają później SSR, kontury, SSS i DoF. Bez niego
/// te efekty musiałyby zgadywać geometrię z samego koloru — a krawędź
/// obiektu o jednolitym albedo jest w kolorze niewidoczna.
struct GBuffer {
    @location(0) color: vec4<f32>,
    @location(1) normal_rough: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(2) color: vec3<f32>,
    // `instance_index` liczy od 1, bo `first_instance` to 1
    @builtin(instance_index) inst: u32,
) -> VsOut {
    let m = models[inst - 1u];
    let world = m.model * vec4<f32>(position, 1.0);

    var out: VsOut;
    out.clip_pos = scene.view_proj * world;
    // macierz modelu jest czysto rotacyjno-skalowa (bez shear), więc
    // wystarczy przemnożyć normalną przez tę samą macierz
    out.world_normal = (m.model * vec4<f32>(normal, 0.0)).xyz;
    out.world_pos = world.xyz;
    // Kolor wierzchołka mnożymy przez tint instancji. Konwersję sRGB
    // robimy RAZ, tutaj: `tint` to wybór koloru drużyny, a nie mapa,
    // więc przeliczamy go raz na wierzchołku, nie na piksel.
    out.color = srgb_to_linear(color) * srgb_to_linear(m.tint.rgb);
    out.uv = uv;

    // Tangent umieszczamy wzdłuż osi X świata w lokalnych wierzchołkach
    // modelu. To NIE jest idealny tangent, ale poprawny: dla UV
    // rozłożonych wzdłuż długości modelu daje właściwe wyniki, a przy
    // sferycznych mapach (koła, kule) różnica jest niewidoczna.
    let t = (m.model * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz;
    out.tangent = t;
    return out;
}

/// Vertex shader passu cienia.
///
/// Zwraca TYLKO pozycję w przestrzeni kamery słońca. Nie ma tu
/// interpolatorów poza `position` — pass nie ma color attachmentu,
/// więc fragment shader w ogóle nie istnieje i GPU zapamiętuje
/// wyłącznie głębokość.
@vertex
fn vs_shadow(
    @location(0) position: vec3<f32>,
    @builtin(instance_index) inst: u32,
) -> @builtin(position) vec4<f32> {
    let m = models[inst - 1u];
    return scene.light_view_proj * m.model * vec4<f32>(position, 1.0);
}

/// Wyjście shadera nieba: pozycja klipu + współrzędne NDC.
struct SkyOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

/// Vertex shader nieba: pełnoekranowy trójkąt bez bufora wierzchołków.
///
/// Nie potrzebujemy geometrii — kierunek promienia odtwarzamy w shaderze
/// fragmentu z `inv_view_proj` i współrzędnych piksela. Trzy wierzchołki
/// dają pełny prostokąt za zero bajtów bufora.
// --- skinning -------------------------------------------------------------
//
// Wierzchołek postaci wpływa na cztery koście jednocześnie. Klasyczne
// „linear blend skinning": pozycję i normalną liczymy jako ważoną sumę
// przekształceń, a nie jako średnią pozycji — różnica widać przy
// zginaniu łokcia, gdzie zwykłe uśrednianie „zwija” rękę.

/// Ważona suma macierzy kości wskazanych przez wierzchołek.
fn skin_matrix(j: vec4<f32>, w: vec4<f32>) -> mat4x4<f32> {
    // W pliku indeksy kości leżą w `JOINTS_0` jako `u16`, ale loader
    // trzyma je jako `f32` (wartości całkowite): w WGSL nie da się
    // czytać `vec4<u32>` z bufora zadeklarowanego jako `Float32x4`
    // bez osobnego formatu w layoucie. Rzutujemy więc tutaj.
    return bones[u32(j.x)] * w.x
        + bones[u32(j.y)] * w.y
        + bones[u32(j.z)] * w.z
        + bones[u32(j.w)] * w.w;
}

/// Wagi znormalizowane do sumy 1.
///
/// Plik gwarantuje sumę ~1, ale eksporterzy bywają niedokładni
/// (0.98 albo 1.03), a złe wagi dają ciemne smugi na krawędziach
/// skóry. Przy całkowicie zerowej wadzie wybieramy pierwszą kość,
/// żeby wierzchołek nie skoczył do początku układu współrzędnych.
fn normalized_weights(w: vec4<f32>) -> vec4<f32> {
    let s = w.x + w.y + w.z + w.w;
    if s < 1e-5 {
        return vec4<f32>(1.0, 0.0, 0.0, 0.0);
    }
    return w / s;
}

/// Vertex shader postaci: skinning w czterech wpływach.
///
/// Wyjście jest **identyczne** z [`vs_main`] — ten sam `VsOut`, to
/// samo `fs_main`. Dzięki temu postać przechodzi dokładnie tą samą
/// ścieżką oświetlenia, co bryły: ta sama rampa anime, ten sam SSS,
/// ten sam materiał. Osobny potok dotyczy wyłącznie tego, skąd biorą
/// się pozycja i normalna.
@vertex
fn vs_skin(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) joints: vec4<f32>,
    @location(3) weights: vec4<f32>,
    @location(4) uv: vec2<f32>,
    // `instance_index` liczy od 1, bo `first_instance` to 1 — ta sama
    // konwencja co w `vs_main`.
    @builtin(instance_index) inst: u32,
) -> VsOut {
    let m = models[inst - 1u];
    let w = normalized_weights(weights);
    // Macierz świata z uwzględnieniem kości: ważona suma przekształceń
    // kości, a NIE średnia pozycji — różnica widać przy zginaniu łokcia.
    let skin = skin_matrix(joints, w);
    let world = m.model * skin * vec4<f32>(position, 1.0);

    var out: VsOut;
    out.clip_pos = scene.view_proj * world;
    // Normalna jest wektorem: zero na końcu zachowuje kierunek
    // i nie wprowadza przesunięcia.
    out.world_normal = normalize((m.model * skin * vec4<f32>(normal, 0.0)).xyz);
    out.world_pos = world.xyz;
    out.color = srgb_to_linear(vec3<f32>(1.0)) * srgb_to_linear(m.tint.rgb);
    out.uv = uv;
    // Tangent jak w `vs_main` — oś X modelu, bo ten asset nie dostarcza
    // własnych tangentów (w `glTF` są opcjonalne i tu ich nie ma).
    out.tangent = (m.model * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz;
    return out;
}

/// Odpowiednik [`vs_skin`] dla passu cieni: zwraca tylko pozycję.
@vertex
fn vs_skin_shadow(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) joints: vec4<f32>,
    @location(3) weights: vec4<f32>,
    @location(4) uv: vec2<f32>,
    @builtin(instance_index) inst: u32,
) -> @builtin(position) vec4<f32> {
    let m = models[inst - 1u];
    let skin = skin_matrix(joints, normalized_weights(weights));
    return scene.light_view_proj * m.model * skin * vec4<f32>(position, 1.0);
}

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    // Identyczna siatka co w `post.wgsl` (`vs_fullscreen`): 3 wierzchołki
    // rozciągnięte poza zakres NDC. `vi / 2` i `vi % 2` dają (0,0), (1,1),
    // (2,0) — a mnożenie przez 4 z odejściem 1 wychodzi poza [-1, 1],
    // więc trójkąt zakrywa cały prostokąt.
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    var out: SkyOut;
    // `z = 1.0` zapisujemy głębokość NIEBIEM. Potok nieba ma wyłączone
    // zapisywanie głębokości i porównanie `LessEqual`, więc niebo
    // przegrywa z każdą bryłą, ale samo niczego nie zasłania.
    out.clip_pos = vec4<f32>(x, y, 1.0, 1.0);
    out.ndc = vec2<f32>(x, y);
    return out;
}

/// sRGB -> liniowy, zgodna z krówką aproksymacją.
///
/// Wszystkie kolory wpisywane z CPU (`Surface::base`, `Lighting`) są
/// w sRGB, bo tak wyglądają w edytorze i tak je wpisuje się w kodzie.
/// Konwersję robimy RAZ, przed oświetleniem — inaczej ACES na końcu
/// dostałbyby kolory wyglądające na 1.5x za jasne.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

/// PCF 3×3: 9 porównań głębokości uśrednionych ze sobą.
///
/// Jeden odczyt daje twardą, „poszlakowaną" krawędź cienia — widoczną
/// przy ruchu kamery jako drganie. Uśrednienie 9 sąsiadów daje gradient
/// szerokości rzędu jednego texela, co wygląda jak miękki pen.
///
/// `bias` odejmujemy od głębokości fragmentu (im bliżej słońca, tym
/// mniejsza głębokość), więc próbka „widzi" obiekt jako minimalnie
/// bliższy i nie dostaje fałszywego cienia na samej sobie.
fn shadow_factor(world_pos: vec3<f32>, N: vec3<f32>) -> f32 {
    // `normal_offset` przesuwa próbkę od powierzchni wzdłuż normalnej.
    // Działa lepiej niż sam depth-bias przy powierzchniach skośnych
    // do kierunku światła, bo zależy od geometrii, a nie od głębokości.
    let offset_pos = world_pos + N * scene.shadow_params.y;
    let lp = scene.light_view_proj * vec4<f32>(offset_pos, 1.0);

    // `w` = 1 dla projekcji ortograficznej, ale dzielimy przez nie
    // świadomie: ten sam kod obsłużyłby perspektywę bez zmian.
    let ndc = lp.xyz / lp.w;
    // NDC -> UV: oś Y odwrócona, bo w NDC rośnie w górę, a na
    // teksturze w dół. Bez tej negacji cień lądowałby pionowo.
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);

    // Poza mapą nie ma sensu zgadywać. Kadr dobieramy do całej sceny
    // (patrz `light_matrix`), więc to zdarza się tylko dla obiektów
    // wystających poza AABB — dla nich zwracamy PEŁNE oświetlenie.
    let in_range = all(uv > vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0))
        && ndc.z >= 0.0 && ndc.z <= 1.0;
    if (!in_range) {
        return 1.0;
    }

    let bias = scene.shadow_params.x;
    // Krok PCF w jednostkach UV. `radius` podany w texelach mnożymy
    // przez rozmiar texela, żeby wynik nie zależał od rozdzielczości.
    let texel = scene.shadow_map_info.x;
    let step = scene.shadow_params.w * texel;
    let ref_depth = ndc.z - bias;

    // 9 próbek w siatce 3×3 wokół środka.
    var sum = 0.0;
    for (var y: i32 = -1; y <= 1; y = y + 1) {
        for (var x: i32 = -1; x <= 1; x = x + 1) {
            let off = vec2<f32>(f32(x), f32(y)) * step;
            sum = sum + textureSampleCompare(shadow_tex, shadow_samp, uv + off, ref_depth);
        }
    }
    return sum / 9.0;
}

/// Rampa dyfuzji anime — serce warstwy rysunkowej.
///
/// Zwraca 0..1 w miejsce zwykłego `N·L` dla warstwy bazowej. NIE dotyka
/// warstwy odbić, która zostaje w pełni fizyczna.
///
/// Dlaczego `smoothstep` a nie `step`: twardy próg daje ostrą, łamliwą
/// granicę, która przy ruchu kamery migocze. Miękki próg daje ten sam
/// rysunkowy efekt „posiomej krawędzi", ale jako gradient szerokości kilku
/// stopni — czytelny i stabilny w animacji.
///
/// `softness` steruje szerokością przejścia: 0.02 to niemal rysunek,
/// 0.6 to zwykłe miękkie PBR. Próg leży na terminatorze (`N·L = 0.5`).
fn toon_ramp(ndl: f32, softness: f32) -> f32 {
    let s = max(softness, 0.005);
    // Dwie stopnie: podstawowa na terminatorze i druga wyżej, żeby duże
    // powierzchnie (ściana, bark) miały czytelną formę, a nie jedną
    // płaską plamę.
    let base = smoothstep(0.5 - s, 0.5 + s, ndl);
    let mid = smoothstep(0.66 - s, 0.66 + s, ndl);
    return base * 0.70 + mid * 0.30;
}

/// Izotropowy rozkład GGX (Trowbridge-Reitz).
///
/// Używany, gdy anizotropia jest pomijalna — jest tańszy i stabilniejszy
/// numerycznie niż wersja z dwiema osiami.
fn distribution_iso(ndh: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = ndh * ndh * (a2 - 1.0) + 1.0;
    return a2 / max(3.14159265 * d * d, 1e-7);
}

/// Anizotropowy rozkład GGX.
///
/// Dwie osie chropowatości (`at`, `ab`) zamiast jednej izotropowej. To
/// daje poświatę rozciągniętą wzdłuż kierunku szczotkowania — ten
/// „jedwabisty", pasmowy połysk na włosach i szczotkowanym metalu,
/// o którym mówi Endfield. Zwykły model izotropowy daje tam okrągłą
/// plamę, która wygląda jak plastik.
fn distribution_aniso(ndh: f32, tdh: f32, bdh: f32, at: f32, ab: f32) -> f32 {
    let a2 = at * ab;
    // `w2` to skrócony wyznacznik normalizujący (Burley / Filament).
    let v = vec3<f32>(ab * tdh, at * bdh, a2 * ndh);
    let w2 = a2 / max(dot(v, v), 1e-8);
    return a2 * w2 * w2 * (1.0 / 3.14159265);
}

/// Fresnel Schlicka — kluczowe dla metalu.
///
/// Wykładnik 5 to dopasowanie do krzywej BRDF, a nie wybór estetyczny:
/// daje `F → 1` pod kątem 90°, czyli obraz odbicia nie gaśnie na
/// krawędziach obiektu.
fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

/// Funkcja widoczności Smitha (izotropowa, z korekcją Heita).
///
/// Anizotropia świadomie NIE wchodzi tutaj. Pełna anizotropowa
/// geometria daje widoczne artefakty na krawędziach, a wizualnie
/// cały efekt „szczotkowania" bierze się z rozkładu `D`. Stąd
/// prawie darmowa anizotropia bez Listków na metalu.
fn visibility_smith(nov: f32, nol: f32, a: f32) -> f32 {
    let a2 = a * a;
    let gv = nol * sqrt(nov * nov * (1.0 - a2) + a2);
    let gl = nov * sqrt(nol * nol * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

/// Otoczenie dwukierunkowe: niebo z góry + odbicie od ziemi z dołu.
///
/// To tanie przybliżenie IBL. Nie jest ray-tracingiem, ale daje to, czego
/// naprawdę brakuje płaskiemu renderowi: obiekt bliski żółtej obudowie
/// maszyny dostaje ciepły odbłysk, a nie szary brak.
///
/// Ta sama funkcja obsługuje potem odbicie lustrzane (patrz
/// `env_specular`), więc diffuse i specular patrzą na to samo
/// „otoczenie" i nie rozjeżdżają się kolorystycznie.
fn ambient_irradiance(N: vec3<f32>, sky: vec3<f32>, bounce: vec3<f32>) -> vec3<f32> {
    // `N.y` w zakresie -1..1 to kąt między normalną a pionem.
    // `smoothstep` zamiast linii, bo surowa linia daje widoczną krawędź
    // na bryłach o zaokrąglonych narożnikach.
    let k = smoothstep(0.0, 1.0, N.y * 0.5 + 0.5);
    return mix(bounce, sky, k);
}

/// Przybliżenie otoczenia dla IBL: jaka radiance dociera z kierunku `dir`.
///
/// Oddzielne od `ambient_irradiance`, bo tam `dir` jest normalną
/// (rozproszona), a tutaj kierunkiem odbicia — dla lustra to zupełnie
/// inny punkt nieba. Bez tego metal w pomieszczeniu odbijałby „uśrednione
/// niebo" zamiast konkretnego, co zabija czytelność.
fn env_radiance(dir: vec3<f32>, sky: vec3<f32>, ground: vec3<f32>) -> vec3<f32> {
    // Ten sam gradient co diffuse, ale o krok ostrzejszy: odbicie
    // pokazuje więcej horyzontu niż rozkład dyfuzyjny.
    let k = smoothstep(-0.35, 0.55, dir.y);
    return mix(ground, sky, k);
}

/// Podział energii IBL: rozkład + odbicie, bez podwójnego liczenia.
///
/// Standardowa sztuczka „split-sum" w wersji analitycznej
/// (Karis, mobile approximation) — dwa wyrazy, które zastępują
/// 256-teksturową tablicę BRDF. Zwraca skalę dla odbicia
/// (`x` = A, `y` = B), gdzie wynik to `f0 * A + B`.
fn env_brdf(f0: vec3<f32>, roughness: f32, nov: f32) -> vec2<f32> {
    // Współczynniki-fit z aproksymacji Karisa — dobrane numerycznie,
    // nie analitycznie, stąd stałe.
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = vec4<f32>(roughness) * c0 + c1;
    // Termin `exp2(-9.28 * nov)` to kątowe stłumienie: pod niskim kątem
    // fresnel rośnie do 1 i odbicie dominuje, co daje biały kontur
    // na krawędziach cylindra.
    let a004 = min(r.x * r.x, exp2(-9.28 * nov)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

/// Przesłonięcie otoczenia przez geometrię (Lagarda).
///
/// Sam `ao` z tekstury mówi „ile światła w ogóle dochodzi", ale nie
/// uwzględnia kąta patrzenia: ściana widziana z boku powinna dostać
/// mniej otoczenia niż stojąca frontem. Ta funkcja skaluje AO
/// fresnelowską poprawką, dzięki czemu narożnik widziany z boku
/// przyciemnia się bardziej niż płaska ściana — dokładnie tak, jak
/// w rzeczywistości.
fn specular_occlusion(nov: f32, ao: f32, roughness: f32) -> f32 {
    // Wykładnik zależy od chropowatości: gładkie powierzchnie
    // odbijają otoczenie pod ostrym kątem, więc są bardziej
    // zależne od AO niż matowe.
    return clamp(pow(nov + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao, 0.0, 1.0);
}

/// Kompensacja energii wielorozproszeniowej.
///
/// Pojedyncze rozproszenie GGX oddaje mniej energii, niż powinno —
/// przy grubym metalu nawet ~20% ginie w głębi modelu. Bez poprawki
/// gruby, jasny metal wygląda jak wycięty z papieru.
///
/// ## Dlaczego `f0`, a nie `f90`
///
/// Pierwsza wersja tej funkcji liczyła `1 + f90·(1/a − 1)`, czyli
/// mnożyła odbicie przez nawet **20×** dla podłogi o chropowatości
/// 0.22. Efekt: cała podłoga zalewała się błękitnym światłem otoczenia
/// i obraz tracił kontrast.
///
/// Poprawna wersja (Filament, Unreal) to:
///
/// ```text
/// compensation = 1 + f0 · (1/E − 1)
/// ```
///
/// gdzie `E` to albedo kierunkowe pojedynczego rozproszenia — jaka
/// część energii faktycznie wychodzi w lobes odbicia. Trzy rzeczy
/// zmieniają wynik na słuszny:
///
///   * mnożnikiem jest **`f0`**, nie `f90`. Dielektryk ma `f0 ≈ 0.04`,
///     więc dostaje ledwie zauważalną poprawkę; metal z `f0 ≈ 0.9`
///     dostaje dużą. To właśnie metale cierpią na utratę energii —
///     rozprasza je wiele odbić, dielektryk prawie jedno.
///   * `E` bierzemy z tego samego dopasowania split-sum co odbicie:
///     `E ≈ A + B`, bo dla środowiska o jednolitej jasności odwrócone
///     odbicie wynosi `f0·A + B`, a dla pełnego odbicia (`f0 = 1`)
///     zostaje właśnie `A + B`.
///   * Gładka powierzchnia ma `E ≈ 1`, więc kompensacja wynosi `1`
///     i nic nie zmienia — czyli nie psuje luster.
fn energy_compensation(f0: vec3<f32>, env_ab: vec2<f32>) -> vec3<f32> {
    // Albedo kierunkowe. `max` chroni przed zerem przy ekstremalnie
    // gładkiej powierzchni, gdzie `A + B` → 0.
    let e = max(env_ab.x + env_ab.y, 1e-2);
    return 1.0 + f0 * (1.0 / e - 1.0);
}

/// Podpowierzchniowe rozpraszanie w przybliżeniu „translucent".
///
/// Nie liczymy tu prawdziwego SSS z mapą grubości — to osobny pass i osobne
/// buforowanie. Zamiast tego modelujemy to, co widać gołym okiem: cienka
/// krawędź (uszy, nos, palce) przepuszcza światło z drugiej strony i robi
/// się ciepła, bo krew pochłania fale zielone.
fn subsurface(N: vec3<f32>, L: vec3<f32>, view_dir: vec3<f32>, mat: Material) -> vec3<f32> {
    // Grubość rozsuwa punkt zbierania transmitancji: cienka skóra
    // przepuszcza światło szeroko, gruba wąsko.
    let thickness = mat.accent.x;
    // Wektor „w głąb" materiału, przesunięty wzdłuż -N.
    let depth_vec = -N * (0.35 + thickness * 0.5);
    let incoming = normalize(L + depth_vec);
    // Transmitancja: jak mocno kierunek do oka zgadza się z kierunkiem
    // wychodzącym wstecz do źródła. Daje poświatę na KRAWĘDZIACH
    // skierowanych do słońca, a nie na środku płaskiej twarzy — co jest
    // dokładnie odwrotnie do klasycznego „ramienia światła".
    let trans = pow(clamp(dot(view_dir, -incoming), 0.0, 1.0), 3.0 + thickness * 6.0);
    // Miękki terminator: skóra nie ma ostrej granicy światła i cienia.
    let wrap = clamp((dot(N, L) + 0.5) / 1.5, 0.0, 1.0);
    return mat.accent.yzw * (trans * 0.85 + wrap * 0.15);
}

/// Mie scattering: wzmocnienie światła w kierunku słońca w mgle.
///
/// Wzór Henyeya-Greensteina z ujemnym `g`, bo rozpraszanie wsteczne
/// (w stronę obserwatora) jest silniejsze niż do przodu — dlatego mgła
/// świeci mocniej, gdy patrzymy W STRONĘ słońca.
fn mie_phase(cos_theta: f32) -> f32 {
    let g = 0.76;
    let g2 = g * g;
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * 3.14159265 * pow(max(denom, 1e-4), 1.5));
}

/// Anizotropia rozpraszania dla mgły.
///
/// ## Po co dwie fazy
///
/// `mie_phase` powyżej ma `g = 0.76` i służy **niebu** — tam chcemy
/// wąski, bardzo wyraźny aureol wokół tarczy słonecznej.
///
/// Mgła gruntowa jest innym ośrodkiem: krople są duże i dobrze mieszane,
/// więc rozpraszanie jest niemal izotropowe. `g = 0.6` (ta sama wartość
/// co `FOG_INSCATTERING_PHASE_G` w WickedEngine) daje mgle łagodne
/// rozjaśnienie w stronę słońca zamiast ostrej plamy.
///
/// WGSL nie ma tu czegoś takiego jak domyślny argument, więc `g`
/// jest jawnym parametrem.
fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    // `max` w mianowniku chroni przed potęgą ujemnej liczby po
    // zaokrągleniu `cos_theta` lekko poniżej -1.
    let denom = 1.0 + g2 - 2.0 * g * cos_theta;
    return (1.0 - g2) / (4.0 * 3.14159265 * pow(max(denom, 1e-4), 1.5));
}

/// Anizotropia rozpraszania mgły gruntowej.
///
/// 0.6 to ta sama wartość, którą WickedEngine używa dla mgły
/// (`FOG_INSCATTERING_PHASE_G` w `fogHF.hlsli`). Nie jest identyczna
/// z `mie_phase` dla nieba — i nie powinna być.
const FOG_PHASE_G: f32 = 0.6;


@fragment
fn fs_main(in: VsOut) -> GBuffer {
    // `normalize(vec3(0))` to NaN, a NaN w kolorze wychodzi czarny —
    // importer dopuszcza zerową normalną, gdy plik jej nie dostarczy albo
    // trójkąt jest zdegenerowany. Dlatego dzielimy ręcznie i podmieniamy
    // wynik na bezpieczny, zamiast przepuszczać NaN do oświetlenia.
    // Robimy to PRZED `N0`, bo WGSL wymaga deklaracji przed użyciem.
    let n_raw = in.world_normal;
    let n_len = length(n_raw);
    let N0 = select(vec3<f32>(0.0, 1.0, 0.0), n_raw / n_len, n_len > 1e-6);

    let V = normalize(scene.eye.xyz - in.world_pos);
    let L = normalize(scene.light_dir.xyz);
    let H = normalize(L + V);

    // --- tekstury
    // V odwracamy: OpenGL/Blender ma V rosnące w górę, wgpu od góry.
    let uv = vec2<f32>(in.uv.x, in.uv.y * mat.flags.w) * mat.params.z;

    // Baza: albo kolor wierzchołka (import `.obj`, gdzie `Kd` jest
    // już w wierzchołkach), albo `Surface::base` (materiały z kodu,
    // gdzie geometria ma białe wierzchołki). `mix` z flagą `base.w`
    // czyta się jak instrukcja — mnożenie przez oba naraz
    // przyciemniłoby model z pliku o kwadrat.
    var albedo = mix(in.color, mat.base.rgb, mat.base.w);
    if (mat.flags.x > 0.5) {
        // tekstura albedo jest w formacie sRGB, więc GPU zdekodował
        // ją do liniowego już przy próbkowaniu — nie robimy tego
        // drugi raz ręcznie
        albedo = albedo * textureSample(albedo_tex, samp, uv).rgb;
    }
    // Mnożnik diffuse przyciemnia bazę BEZ ruszania emisji — dzięki
    // temu ciemna neonowa listwa może mocno świecić.
    albedo = albedo * mat.tint.w;

    // --- normal mapping
    //
    // Bazę (T, B) liczymy ZAWSZE, także bez mapy normalnych: anizotropia
    // i SSS potrzebują bitangi, a budowanie jej tylko w gałęzi z mapą
    // dawałoby dwie różne ścieżki do tego samego wyniku.
    var T = in.tangent - N0 * dot(N0, in.tangent);
    let t_len = length(T);
    let tangent_ok = t_len > 1e-5;
    if (tangent_ok) {
        T = T / t_len;
    }
    let B = cross(N0, T);
    var N = N0;
    if (mat.params.w > 0.0 && tangent_ok) {
        let ts = textureSample(normal_tex, samp, uv).rgb * 2.0 - vec3<f32>(1.0, 1.0, 1.0);
        N = normalize(N0 + (T * ts.x + B * ts.y) * mat.params.w);
    }

    // --- ORM: R = AO, G = chropowatość, B = metaliczność
    let orm = textureSample(orm_tex, samp, uv).rgb;
    let ao = mix(1.0, orm.r, mat.flags.z);
    let roughness = clamp(mix(mat.params.x, orm.g, mat.flags.z), 0.03, 1.0);
    let metallic = clamp(mix(mat.params.y, orm.b, mat.flags.z), 0.0, 1.0);

    let ndotl = max(dot(N, L), 0.0);
    let ndotv = max(dot(N, V), 1e-4);

    // ==================== WARSTWA 1: baza rysunkowa ====================
    //
    // To jest ta część, która robi wrażenie „anime". Zamiast ciągłego
    // `N·L` przepuszczamy go przez rampę, a skalę mieszamy z ustawieniem
    // materiału (`surface.x`) i parametrem globalnym sceny. Dzięki temu
    // jedna klatka może mieć rysunkową postać i fizyczne otoczenie.
    let stylize = clamp(mat.surface.x * scene.stylize.x, 0.0, 1.0);
    let ndl_shaped = mix(ndotl, toon_ramp(ndotl, scene.stylize.y), stylize);
    let ndl_final = max(ndl_shaped, 0.0);

    // ==================== WARSTWA 2: pełne PBR ====================
    //
    // Podział energii jak w rozdzielonym GGX: dyfuzja dostaje
    // `(1-F)·(1-metal)`, reszta idzie do odbicia. Bez tego metal
    // odbijałby jeszcze światło dyfuzyjne i wyglądał jak plastik
    // pomalowany na srebro.
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // Anizotropia: dwie osie chropowatości rozciągnięte ±anizotropia.
    // Trzymamy je w bezpiecznym minimum, bo `a → 0` daje w GGX
    // nieskończoność, czyli lśniący pojedynczy piksel.
    let aniso = clamp(mat.surface.y, -0.95, 0.95);
    let a_base = roughness * roughness;
    let at = max(a_base * (1.0 + aniso), 0.002);
    let ab = max(a_base * (1.0 - aniso), 0.002);

    let ndh = max(dot(N, H), 0.0);
    let vdh = max(dot(V, H), 0.0);

    // Rozkład: anizotropowy tylko gdy różnica osi jest zauważalna.
    // Poniżej progu izotropowy jest tańszy i stabilniejszy numerycznie.
    var d_term: f32;
    if (abs(aniso) > 0.01) {
        d_term = distribution_aniso(ndh, dot(T, H), dot(B, H), at, ab);
    } else {
        d_term = distribution_iso(ndh, a_base);
    }
    let g_term = visibility_smith(ndotv, max(ndotl, 1e-4), a_base);
    let f_term = fresnel_schlick(vdh, f0);

    let specular = d_term * g_term * f_term;
    let kd = (vec3<f32>(1.0) - f_term) * (1.0 - metallic);
    // Rampa wchodzi TUTAJ — do warstwy bazowej. Na wejściu jest
    // liniowa, więc podział energii pozostaje poprawny.
    let diffuse = kd * albedo * (ndl_final / 3.14159265);

    let sun = srgb_to_linear(scene.light_color.rgb) * scene.light_color.a;

    // --- cień: przyciemniamy TYLKO światło bezpośrednie
    //
    // Światło otoczenia zostaje nietknięte — inaczej cień byłby czarny
    // i nie dojrzałby żadnego detalu. W cieniu widać błękit nieba i
    // rozproszone światło, dokładnie jak w rzeczywistości.
    //
    // Mnożymy przez `ndotl` (ciągłe), NIE przez rampę: rampowany cień
    // pozostawałby nierównomierny i „skakałby" po obrysie bryły.
    //
    // `shadow_map_info.y` to włącznik. Gdy cienie są wyłączone, skip
    // całej próbki — nie tylko mnożenia, bo samo `textureSampleCompare`
    // kosztuje tyle, co kilkanaście ALU.
    let direct_unshadowed = (diffuse + specular) * sun * ndotl;
    var direct = direct_unshadowed;
    if (scene.shadow_map_info.y > 0.5) {
        let lit = shadow_factor(in.world_pos, N0);
        // `strength` pozwala mieć cienie mocne albo ledwo widoczne
        // bez zmiany mapy — przydatne, gdy słońce jest nisko.
        let shadow = mix(1.0, lit, scene.shadow_params.z);
        direct = direct_unshadowed * shadow;
    }

    // --- podpowierzchniowe rozpraszanie
    //
    // Mnożymy przez `ndotl`, żeby SSS znikł na powierzchniach odwróconych
    // od słońca — inaczej w nocy cała postać byłaby różowa. `view_dir`
    // to kierunek OD oka (V w shaderze wskazuje do oka).
    var sss = vec3<f32>(0.0);
    if (mat.surface.z > 0.0) {
        sss = subsurface(N0, L, -V, mat) * sun * mat.surface.z * ndotl;
    }

    // --- otoczenie (IBL)
    //
    // Otoczenie NIE przechodzi przez rampę: niebo oświetla obiekt ze
    // wszystkich stron i rampa nie ma tu czego rysować. Gdyby ją tu
    // wcisnęliśmy, górne partie zawsze byłaby jasne.
    //
    // --- DLACZEGO TO ROZBITE NA DWA SKŁADNIKI ---
    //
    // Wcześniej diffuse i specular brały jedno i to samo `irradiance`.
    // Efekt był taki, że metal dostawał tyle samo otoczenia co plastik,
    // a gładka powierzchnia nie odbijała absolutnie niczego — metal
    // stawał się matowy i ciemny, czyli „plastikowy".
    //
    // Poprawnie (Fdez-Agüera, tak jak w UE) otoczenie ma DWA wejścia:
    //   * **diffuse irradiance** — rozkład po połowie sfery, zależy od `N`,
    //   * **specular radiance** — LUSTRO odbicia `reflect(-V, N)`,
    //     czyli z zupełnie innego miejsca otoczenia niż diffuse.
    //
    // Dla kuli obie wartości wyraźnie się różnią; dla płaskiej ściany
    // różnica jest mała. Właśnie ta różnica daje metalowi „charakter".
    let sky = srgb_to_linear(scene.ambient_time.rgb);
    // Bounce: odbicie od ziemi. Przyjmujemy je jako niebo przesunięte
    // w dół i ocieplone — pełna analiza GI wymagałaby osobnego passu,
    // a efekt jest ledwie widoczny, a koszt stałby przy każdej bryle.
    let bounce = sky * vec3<f32>(0.62, 0.58, 0.52);
    let irradiance = ambient_irradiance(N0, sky, bounce);
    // Ambiencję mnożymy przez albedo i AO — inaczej wklęsłości byłyby
    // jaśniejsze od wypukłości.
    let ao_ambient = ao * (1.0 - metallic * 0.75);
    let ambient = irradiance * albedo * ao_ambient;

    // --- specular IBL: otoczenie w kierunku odbicia
    //
    // To największa różnica wobec „płaskiego" renderu: metal wreszcie
    // COŚ odbija. Kierunek odbicia liczymy po normal mappingu, więc
    // faktura falowa też się odbija i daje iskrzenie na blaskach.
    let R = reflect(-V, N);
    let radiance = env_radiance(R, sky, bounce);
    // Split-sum: `f0 * A + B` z analitycznego dopasowania Karisa.
    // Ta sama formuła obsługuje dielektryk i metal — różni je `f0`.
    //
    // Nazwa `env_split`, nie `ab`: `ab` to już oś anizotropii
    // w tym shaderze (patrz `distribution_aniso`), a WGSL zabrania
    // ponownej deklaracji nazwy w tym samym zakresie.
    let env_split = env_brdf(f0, roughness, ndotv);
    // Kompensacja energii: gruby metal bez niej traci nawet 20%
    // energii i wygląda jak wycięty z papieru.
    let ecomp = energy_compensation(f0, env_split);
    // Przesłonięcie fresnelowskie: narożnik widziany z boku
    // przyciemnia się bardziej niż płaska frontowa ściana.
    let so = specular_occlusion(ndotv, ao, roughness);
    // Odejmujemy dyfuzję, która już weszła do `ambient` — inaczej
    // odbicie podwójnie liczyłoby kolor materiału.
    let specular_ibl = max(
        radiance * (f0 * env_split.x + vec3<f32>(env_split.y)) * so * ecomp
            - radiance * albedo * ao_ambient,
        vec3<f32>(0.0),
    );

    // --- światło krawędziowe (rim)
    //
    // Odbicie otoczenia widziane pod kątem — najmocniejsze tam, gdzie
    // normalna jest prostopadła do kierunku patrzenia. W odróžnieniu
    // od konturu to jest prawdziwy fenomen świetlny: krawędź obiektu
    // łapie światło z boku, którego środek w ogóle nie widzi.
    var rim = vec3<f32>(0.0);
    if (mat.surface.w > 0.0) {
        let rim_term = pow(1.0 - ndotv, 3.0) * mat.surface.w;
        // Kolor bierze z otoczenia, nie z albedo — dzięki temu poświat
        // jest chłodna w cieniu i ciepła na słońcu.
        rim = irradiance * rim_term * (0.4 + 0.6 * ndotl);
    }

    // --- światło własne
    let emissive = mat.tint.rgb;

    // --- mgła wysokościowa ---------------------------------------------
    //
    // Zamiast `1 - exp(-dist · ρ)` liczymy **analityczny całkowity**
    // gęstości wzdłuż promienia oka. Gęstość mgły rośnie z wysokością
    // (zanika wyżej), a promień często przebiega skośnie, więc wzdłuż
    // niego trzeba scałkować `exp(-h/H)`. Wzór analityczny pochodzi z
    // iquilezles.org i jest ten sam, którego używa WickedEngine
    // w `fogHF.hlsli` (`GetFogAmount`).
    //
    // Co to zmienia w obrazie:
    //   * mgła **zbiera się u dołu** — horyzont jest gęstszy niż
    //     powietrze nad głową. Poprzedni wzór dawał jednolitą
    //     „mleczną" warstwę na każdej wysokości, przez co odległe
    //     budynki ginęły tak samo niezależnie od tego, czy stoją
    //     na ziemi, czy wiszą w powietrzu;
    //   * patrzenie w słońce **rozjaśnia** mgłę, a w bok nie — przez
    //     fazę Henyeya-Greensteina. Wcześniej ten efekt był, ale jako
    //     stały mnożnik `2.4`, niezależny od kąta patrzenia.
    //
    // `scene.fog.x` to wysokość zanikania mgły. Reszta struktury
    // `atmos` (x = gęstość, yzw = kolor) zostaje jak była.
    let O = scene.eye.xyz;
    let to_frag = in.world_pos - O;
    let dist = length(to_frag);
    // Nazwa `V` jest już zajęta w `fs_main` (kierunek do oka), więc
    // kierunek promienia nazywamy inaczej — WGSL nie przepuszcza
    // ponownej definicji w tym samym zakresie.
    let ray_dir = to_frag / max(dist, 1e-4);

    // `scene.fog.x` to wysokość zanikania mgły. Gdy wynosi 0 (albo jest
    // ujemna z powodu błędu w konfiguracji), wypadamy w jednolity
    // przypadek: gęstość stała wzdłuż całego promienia. To zachowanie
    // sprzed wprowadzenia mgły wysokościowej, więc stare sceny wyglądają
    // dokładnie tak samo.
    var fog_amount: f32;
    let h_end = scene.fog.x;
    if (h_end <= 0.01) {
        // Jednolita gęstość: `1 - e^(-ρ·d)`.
        fog_amount = 1.0 - exp(-dist * scene.atmos.x);
    } else {
        // `6.907755 = ln(1000)`: tyle potrzeba, żeby gęstość spadła
        // tysiąckrotnie na wysokości `h_end`.
        let fog_falloff = 6.907755 / h_end;

        let origin_h = O.y;
        let vz = ray_dir.y;
        // `abs()` chroni przed dzieleniem przez ~0, czyli patrzeniem
        // idealnie poziomo — wtedy całkowita jest graniczna.
        let effective_z = max(abs(vz), 0.001);

        // Wysokość końca promienia: izolacja y z równania parametrycznego
        // prostej `O + t·V`.
        let end_h = dist * vz + origin_h;
        // Część promienia poniżej dolnej granicy ma gęstość stałą, więc
        // daje się scałkować zwykłym mnożeniem przez długość.
        let min_h = min(origin_h, end_h);
        let base_distance = clamp(-min_h / effective_z, 0.0, dist);
        let exp_distance = dist - base_distance;
        let height_falloff = max(min_h, 0.0);

        // Analityczna całkowita części eksponencjalnej — ten sam wzór co
        // w WickedEngine (`fogHF.hlsli`, `GetFogAmount`), przy dolnej
        // granicy gęstości równej 0.
        let integral = exp(-height_falloff * fog_falloff)
            * (1.0 - exp(-exp_distance * effective_z * fog_falloff))
            / (effective_z * fog_falloff);

        let optical_depth = scene.atmos.x * (base_distance + integral);
        fog_amount = 1.0 - exp(-optical_depth);
    }

    // Faza Henyeya-Greensteinna: rozpraszanie jest silne wprost na
    // słońce i słabe w przeciwnym kierunku. To ona nadaje mgle
    // kierunek — bez niej mgła jest szarą folią niezależnie od tego,
    // gdzie jest słońce.
    // Faza Henyeya-Greensteinna: rozpraszanie jest silne wprost na
    // słońce i słabe w przeciwnym. To ona nadaje mgle kierunek — bez
    // niej mgła jest szarą folią niezależnie od tego, gdzie słońce.
    //
    // Używamy `ray_dir`, nie `V`: `V` w `fs_main` to kierunek **do oka**,
    // a tu potrzebujemy kierunku **od oka**. To wektory przeciwne — przy
    // rzucie w stronę słońca dałyby odwrotną odpowiedź i mgła świeciłaby
    // w złym miejscu.
    let hg = henyey_greenstein(dot(-ray_dir, L), FOG_PHASE_G);
    let fog_color = srgb_to_linear(scene.atmos.yzw) * (1.0 + hg * 2.4);

    // `specular_ibl` jest osobnym składnikiem, bo liczy odbicie
    // otoczenia w miejscu, którego diffuse w ogóle nie bierze pod uwagę.
    var lit = direct + ambient + specular_ibl + rim + sss + emissive;
    // Sufit 0.92: przy pełnej gęstości zostawiamy resztę, bo obiekt
    // całkowicie wtopiony w mgłę wygląda jak błąd, a nie jak głębia.
    lit = mix(lit, fog_color, clamp(fog_amount, 0.0, 0.92));

    // ==================== G-BUFFER ====================
    //
    // Normalną zapisujemy w przestrzeni oka: wektor do oka jest tam
    // zwykle `(0,0,-1)`, więc odbicie lustrzane w SSR to jedno
    // `reflect`, bez transformacji do świata i z powrotem.
    let N_view = normalize((scene.view * vec4<f32>(N, 0.0)).xyz);
    var out: GBuffer;
    out.color = vec4<f32>(lit, 1.0);
    // `w` = chropowatość, bo SSR musi wiedzieć, jak bardzo rozmyć
    // odbicie: lustro (0.05) ma być ostre, mat (0.9) rozlane.
    out.normal_rough = vec4<f32>(N_view, roughness);
    return out;
}

/// Shader nieba: fizyczne rozpraszanie Rayleigh + Mie.
///
/// ## Model
///
/// Atmosfera to warstwa o wykładniczym spadku gęstości. Promień
/// przechodzi przez nią i zbiera rozproszenie w dwóch składnikach:
///
/// * **Rayleigh** — rozpraszenie na molekułach powietrza. Zależy
///   silnie od długości fali (`1/λ⁴`), stąd niebo jest niebieskie:
///   fiolet i błękit rozpraszają się najmocniej, a oko najmniej
///   czuje fiolet.
/// * **Mie** — rozpraszenie na kroplach wody i pyłach. Praktycznie
///   niezależne od długości fali, więc odpowiada za białą mgłę wokół
///   słońca i za poświatę horyzontu.
///
/// Liczymy to analitycznie (Preetham w uproszczeniu), bez ray-marchingu:
/// pełne rozpraszanie wielokierunkowe kosztowałoby kilkadziesiąt próbek
/// na piksel, a przy statycznym słońcu różnica jest widoczna dopiero
/// w atlasie chmur, którego i tak tu nie ma.
@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // Promień przez piksel: NDC -> świat przez odwrotność `view_proj`.
    // Osi Y NIE odwracamy tutaj — `inv_view_proj` sama poprawnie obsługuje
    // układ współrzędnych, bo operujemy w NDC, a nie w UV.
    let far_p = scene.inv_view_proj * vec4<f32>(in.ndc.x, in.ndc.y, 1.0, 1.0);
    let near_p = scene.inv_view_proj * vec4<f32>(in.ndc.x, in.ndc.y, 0.0, 1.0);
    let dir = normalize(far_p.xyz / far_p.w - near_p.xyz / near_p.w);

    let L = normalize(scene.light_dir.xyz);
    let up = clamp(dir.y, -1.0, 1.0);
    // Względna masa powietrza wzdłuż promienia: rośnie przy niskim
    // słońcu (dłuższa droga) — stąd czerwone zachody.
    let zenith = max(up, 0.0);
    let air_mass = 1.0 / (zenith + 0.15);

    // Rayleigh: βR dla λ = (680, 550, 440) nm, przeskalowane przez skalę
    // wysokości. Wartości dobrane tak, żeby niebieski kanał
    // dominował o rząd wielkości — to jest powód niebieskości nieba.
    let beta_r = vec3<f32>(5.8e-6, 13.5e-6, 33.1e-6) * 8000.0;
    // Mie: βM jednowartościowe (nie zależy od barwy — stąd biała mgła).
    let beta_m = vec3<f32>(21e-6 * 1200.0);

    let cos_theta = dot(dir, L);
    // Rayleigh silnie do przodu (symetryczny w obie strony).
    let phase_r = 3.0 / (16.0 * 3.14159265) * (1.0 + cos_theta * cos_theta);
    // Mie: wąskie maksimum w kierunku słońca, stąd aureola wokół tarczy.
    let g = 0.76;
    let g2 = g * g;
    let phase_m = (1.0 - g2)
        / (4.0 * 3.14159265 * pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5));

    // Tłumienie: im dłuższa droga, tym więcej światła ginie.
    // Ten sam `air_mass` dla obu składników, bo rozkład gęstości obu
    // jest wykładniczy.
    let extinction = exp(-(beta_r + beta_m) * air_mass);
    let inscatter = (beta_r * phase_r + beta_m * phase_m) * air_mass;

    // Surowe `inscatter` to wartości rzędu 1e-2, a scena pracuje
    // w jednostkach, gdzie słońce ma natężenie 4.0 — stąd skalowanie.
    var sky = inscatter * extinction * 12.0;
    // Rozpraszanie jest silniejsze przy horyzoncie, bo tam promień
    // przechodzi przez najwięcej powietrza. Bez tego ten pasek byłby
    // niewidoczny, a mgła zlewałaby się z niebem w jedną szarość.
    let horizon_glow = pow(1.0 - zenith, 6.0) * 0.35;
    sky = sky + srgb_to_linear(vec3<f32>(0.55, 0.68, 0.85)) * horizon_glow;

    // --- tarcza słoneczna
    //
    // Słońce jest bardzo jasne (dziesiątki jednostek), bo właśnie potem
    // bloom i anamorficzna poświata dostają materiał do pracy.
    let sun_angular = 0.0093; // ~0.53° średnica kątowa Ziemi
    let sun_angle = acos(clamp(cos_theta, -1.0, 1.0));
    let sun_disk = smoothstep(sun_angular, sun_angular * 0.6, sun_angle);
    // Poświata wokół tarczy — osobno od dysku, bo to ona (a nie dysk)
    // zamienia źródło w poświatę w bloomie.
    let sun_glow = pow(max(cos_theta, 0.0), 180.0) * 2.0;
    let sun = srgb_to_linear(scene.light_color.rgb) * (sun_disk * 90.0 + sun_glow * 6.0);

    var color = sky + sun;
    // Pod horyzontem zostawiamy ciemny brzeg. Niebo ma być widoczne tylko
    // nad horyzontem; poniżej i tak zakrywa je teren, a jaśniejący brzeg
    // psułby sylwetki obiektów w oddali.
    let below = smoothstep(-0.12, 0.02, up);
    color = mix(color * 0.35, color, below);

    return vec4<f32>(max(color, vec3<f32>(0.0)), 1.0);
}
