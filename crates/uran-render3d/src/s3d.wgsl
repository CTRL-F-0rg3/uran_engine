// Shader 3D — PBR z teksturami (albedo, normalne, ORM) i oświetleniem.
//
// Model oświetlenia: rozdzielone GGX (Cook-Torrance) z jednym
// światłem kierunkowym i otoczeniem hemisferycznym. To najtaniej
// PBR, jakie daje wiarygodny obraz: metaliczność i chropowatość
// naprawdę zmieniają wygląd, a nie tylko cieniują.

struct Scene {
    // Uważaj na wyrównanie w WGSL: samo `vec3<f32>` zajmuje 12 B, ale
    // następna zmienna musi zaczynać się od wielokrotności 16.
    // Dlatego WSZĘDZIE są `vec4`: 208 B łącznie z macierzą cienia.
    view_proj: mat4x4<f32>,     // 64 B
    eye: vec4<f32>,             // xyz = pozycja oka, w = nieużywane
    light_dir: vec4<f32>,       // xyz = kierunek światła, w = nieużywane
    light_color: vec4<f32>,     // rgb = kolor światła, a = intensywność
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
    out.color = color * m.tint.rgb;
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
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
    let uv = vec2<f32>(in.uv.x, in.uv.y * mat.flags.w);

    var albedo = in.color;
    if (mat.flags.x > 0.5) {
        // tekstura albedo jest w formacie sRGB, więc GPU zdekodował
        // ją do liniowego już przy próbkowaniu — nie robimy tego
        // drugi raz ręcznie
        albedo = albedo * textureSample(albedo_tex, samp, uv).rgb;
    }

    // --- normal mapping
    var N = N0;
    if (mat.params.w > 0.0) {
        let ts = textureSample(normal_tex, samp, uv).rgb * 2.0 - vec3<f32>(1.0, 1.0, 1.0);
        // ortonormalna baza (n, t, b) — Duffinbrin. Bez niej tangent
        // musiałby być znormalizowany i odznaczony ręcznie, a każda
        // niortogonalność dawałaby śmietne poświaty.
        let Nn = N0;
        var T = in.tangent - Nn * dot(Nn, in.tangent);
        let len = length(T);
        if (len > 1e-5) {
            T = T / len;
            let B = cross(Nn, T);
            N = normalize(Nn + (T * ts.x + B * ts.y) * mat.params.w);
        }
    }

    // --- ORM: R = AO, G = chropowatość, B = metaliczność
    let orm = textureSample(orm_tex, samp, uv).rgb;
    let ao = mix(1.0, orm.r, mat.flags.z);
    let roughness = clamp(mix(mat.params.x, orm.g, mat.flags.z), 0.03, 1.0);
    let metallic = clamp(mix(mat.params.y, orm.b, mat.flags.z), 0.0, 1.0);

    let ndotl = max(dot(N, L), 0.0);
    let ndotv = max(dot(N, V), 1e-4);

    // --- BRDF: rozdzielone GGX
    //
    // Rozdzielenie diffuse i specular (k_*) jest konieczne dla
    // poprawnej energetyki: bez tego metal odbijalby światło
    // dyfuzyjne i wyglądał jak plastik pomalowany na srebro.
    let a = roughness * roughness;
    let a2 = a * a;

    let ndh = max(dot(N, H), 0.0);
    let ndv = max(dot(N, V), 0.0);
    let vdh = max(dot(V, H), 0.0);

    // rozkład GGX (normalny)
    let d_term = a2 / max(3.14159265 * pow(ndh * ndh * (a2 - 1.0) + 1.0, 2.0), 1e-6);
    // geometria Smitha z korygencą Schlicka
    let k = a * 0.5;
    let gv = ndv / (ndv * (1.0 - k) + k);
    let gl = ndotl / (ndotl * (1.0 - k) + k);
    let g_term = gv * gl;
    // Fresnel Schlicka — kluczowe dla metalu
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let f_term = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vdh, 5.0);

    let specular = d_term * g_term * f_term / (4.0 * ndv * max(ndotl, 1e-4));
    // energia dyfuzyjna: (1 - F) * (1 - metaliczność)
    let kd = (vec3<f32>(1.0) - f_term) * (1.0 - metallic);
    let diffuse = kd * albedo / 3.14159265;

    let sun = srgb_to_linear(scene.light_color.rgb) * scene.light_color.a;
    // UWAGA: WGSL nie ma `mut` dla `let` — zmienne sa z natury
    // mutowalne, a `mut` jest zarezerwowanym slowem kluczowym
    // (uzywanym np. w `fn`/`struct`). Dlatego zamiast modyfikowac
    // `direct` w miejscu, liczymy cien i przypisujemy wynik raz.
    let direct_unshadowed = (diffuse + specular) * sun * ndotl;

    // --- cień: przyciemniamy TYLKO światło bezpośrednie
    //
    // Światło otoczenia zostaje nietknięte — inaczej cień na trawie
    // byłby czarny, a w lesie prawie niewidoczny (otoczenie w lesie
    // jest silniejsze niż na otwartym polu). Dopiero `mix` daje
    // sensowne cienie: w cieniu widać błękit nieba i rozproszone
    // światło, dokładnie tak jak w rzeczywistości.
    //
    // `shadow_map_info.y` to włącznik. Gdy cienie są wyłączone, skip
    // całej próbki — nie tylko mnożenia, bo samo `textureSampleCompare`
    // kosztuje tyle, co kilkanaście ALU.
    var direct = direct_unshadowed;
    if (scene.shadow_map_info.y > 0.5) {
        let lit = shadow_factor(in.world_pos, N);
        // `strength` pozwala mieć cienie mocne albo ledwo widoczne
        // bez zmiany mapy — przydatne, gdy słońce jest nisko i cień
        // musi być miękki, żeby nie dominował nad sceną.
        let shadow = mix(1.0, lit, scene.shadow_params.z);
        direct = direct_unshadowed * shadow;
    }

    // --- otoczenie: hemisfera z nieba u góry i odbiciem ziemi
    let sky = srgb_to_linear(scene.ambient_time.rgb);
    // Zakres 0.30..1.05, nie 0.45..1.25: przy wyższym górnym
    // graniczeniu ambient zjadał kontrast i cała scena wyglądała
    // jak przemywana (płaski teren, brak cieni). Światło otoczenia ma
    // podnosić cienie do poziomu otoczenia, a nie zastępować słońce.
    let hemi = mix(sky * 0.30, sky * 1.05, N.y * 0.5 + 0.5);
    // Ambiencję mnożymy przez albedo i AO — inaczej wklęsłości
    // byłyby jaśniejsze od wypukłości.
    let ambient = hemi * albedo * ao * (1.0 - metallic * 0.75);

    // --- światło wsteczne: rozjaśnia krawędzie skierowane do kamery
    let rim = pow(1.0 - ndotv, 4.0) * 0.07;
    var lit = direct + ambient + albedo * rim;

    // --- mgła: dalekie obiekty zlewają się z niebem
    let dist = length(scene.eye.xyz - in.world_pos);
    let fog = clamp((dist - 60.0) / 260.0, 0.0, 0.88);
    lit = mix(lit, srgb_to_linear(vec3<f32>(0.45, 0.60, 0.82)), fog);

    return vec4<f32>(lit, 1.0);
}

/// sRGB -> liniowy, zgodną z krówką aproksymacją.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}
