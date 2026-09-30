// Post-processing sceny 3D w stylu Endfield.
//
// Wejście jest LINIOWE i HDR (rgba16float), wyjście to powierzchnia gry.
// Tonemapping w tej dwojstce jest konieczny — bez niego wartości powyżej
// 1.0 obcinałyby się w 8 bitach i obraz był płaski.
//
// ## Kolejność passów
//
// ```
//   hdr + gbuffer + depth
//        ├─► fs_ssr        ─► ssr_tex
//        ├─► fs_godray     ─► godray_tex   (promienie słoneczne)
//        └─► fs_composite ◄─ wszystkie powyżej + bloom
// ```
//
// Kolejność ma znaczenie: SSR czyta surowy HDR (bez odbicia, żeby nie
// liczyć odbicia odbicia), a kompozycja składa wszystko w jednym
// miejscu. Dzięki temu jest JEDEN kosztowny pass na piksel zamiast
// osobnego na każdy efekt.

struct Params {
    // x = ekspozycja, y = siła bloom, z = winieta, w = aberracja
    grade: vec4<f32>,
    // x = próg bloom, y = ziarno, z = saturacja, w = kontrast
    fx: vec4<f32>,
    // x,y = rozmiar kadru, z = czas, w = siła AA
    screen: vec4<f32>,
    // --- nowe grupy ---------------------------------------------------
    // x,y = pozycja słońca w UV, z = widoczność (0 = poza kadrem),
    // w = promień tarczy w UV (do flary)
    sun: vec4<f32>,
    // x = siła konturu, y = grubość w px, z = próg głębokości,
    // w = czułość na jasność (kontur zanika w świetle)
    outline: vec4<f32>,
    // x = siła SSR, y = maks. odległość w jednostkach świata,
    // z = grubość, w = minimalna chropowatość dla odbicia
    ssr: vec4<f32>,
    // x = siła SSS, y = promień w px, z = siła DoF, w = ogniskowa
    // odległość w jednostkach świata
    scatter: vec4<f32>,
    // x = przysłona (aperture) DoF, y = maks. blur w px,
    // z = anamorficzna siła, w = siła flary
    lens: vec4<f32>,
    // x = siła god rays, y = gęstość, z = dekodowanie (0 sRGB / 1 lin),
    // w = rezerwa
    rays: vec4<f32>,
    // x,y = near, far — do liniaryzacji głębokości
    depth: vec4<f32>,
    // x = tint cieni (podbicie RGB), y = tint świateł (RGB),
    // z = siła podziału tonów, w = temperatura (błękit ↔ bursztyn)
    grade2: vec4<f32>,
    // x = siła SSAO, y = promień (m), z = bias (m), w = intensywność
    ssao: vec4<f32>,
    // x = tan(FOV/2) w pionie, y = aspect, z,w = rezerwa.
    // SSAO odtwarza z tego pozycję w METRACH.
    proj: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
// G-Bufer i głębokość są potrzebne tylko w passach ekranowych
// (SSR, kontury, DoF, SSS). Kompozycja czyta je, a bright/blur — nie.
@group(0) @binding(4) var gbuf_tex: texture_2d<f32>;
@group(0) @binding(5) var depth_tex: texture_depth_2d;
// Dwa osobne cele dla efektów ekranowych. NIE da się ich współdzielić:
// każdy pass nadpisałby wynik poprzedniego, a kompozycja czyta
// oba naraz (odbicie + poświat słoneczna).
@group(0) @binding(6) var ssr_tex: texture_2d<f32>;
@group(0) @binding(7) var rays_tex: texture_2d<f32>;
// Mapa SSAO. Osobny binding (a nie współdzielona z `rays_tex`) slot),
// bo ma inny format: `R8Unorm` zamiast `Rgba16Float`. Jeden wpis w
// layoucie = jeden format, więc dwie tekstury o różnych formatach
// muszą mieć osobne sloty.
@group(0) @binding(8) var ao_tex: texture_2d<f32>;

// --- pełnoekranowy trójkąt: 3 wierzchołki, bez bufora ---------------
struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> VsOut {
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    var out: VsOut;
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    // UWAGA na oś Y: w NDC `y = -1` to DÓŁ kadru, a w teksturze
    // `v = 0` to GÓRA wiersza. Bez `1.0 - y` dolna część obrazu
    // pobierałaby górną i cała scena byłaby odbita pionowo.
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// --- AgX -----------------------------------------------------------------
// Tonemapping używany we współczesnych silnikach (Unreal 5, Blender 4).
//
// Zwykły ACES (Narkowicz) jest szybki, ale ma dwie wady, które WIDAĆ na
// obrazie:
//   * gubi szczegóły w cieniach — mocno przyciemnia do zera,
//   * przy bardzo jasnym, nasyconym źródle daje pastiszową, jednolitą
//     plamę zamiast płynnej desaturacji do bieli.
//
// AgX naprawia obie. Przebieg:
//   1. macierz rektyfikacji — linear RGB na przestrzeń percepcyjną,
//   2. `log2` i normalizacja do zakresu [0,1] (kontrast z logu),
//   3. krzywa S (wielomian) — miękkie przejścia tonalne,
//   4. „look": nasycenie i kontrast,
//   5. macierz odwrotna + `pow(2.2)` — wrót do wartości LINIOWYCH.
//
// Ten ostatni krok jest ważny dla tego renderera: powierzchnia jest
// `Rgba8UnormSrgb`, więc sRGB koduje sprzęt. Gdybyśmy zakodowali ją
// w shaderze, obraz byłby prześwietlony (podwójny gamma).

/// Zakres ekspozycji AgX — dolna i górna granica log2 EV.
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

/// Macierz rektyfikacji: linear sRGB -> przestrzeń percepcyjna.
fn agx_transform() -> mat3x3<f32> {
    // Kolumny, nie wiersze — WGSL buduje macierz z kolumn.
    return mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336000000000),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
}

/// Macierz odwrotna rektyfikacji (po krzywej tonalnej).
fn agx_transform_inv() -> mat3x3<f32> {
    return mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
}

/// Krzywa S AgX — wielomian 6. stopnia.
///
/// Wielomian, a nie `smoothstep`, bo daje wypukłość w środku zakresu:
/// to one rozciągają odcienie, zamiast zwijać je w czarną plamę.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x3 = x2 * x;
    let x4 = x2 * x2;
    let x5 = x4 * x;
    let x6 = x3 * x3;
    return 15.5 * x6
        - 40.14 * x5
        + 31.96 * x4
        - 6.868 * x3
        + 0.4298 * x2
        + 0.1191 * x
        - 0.00232;
}

/// „Look" po krzywej tonalnej: nasycenie + delikatny gamma.
///
/// AgX celowo odbarwia jasne partie (żeby światła nie były kolorowymi
/// plamami). Przywracamy trochę nasycenia, bo obraz ma być
/// „rysunkowy" — bez tego neonowe listwy wyglądają jak szare smugi.
fn agx_look(v: vec3<f32>) -> vec3<f32> {
    let lw = vec3<f32>(0.2126, 0.7152, 0.0722);
    let luma = dot(v, lw);
    // `power` 1.15 rozjaśnia środek zakresu bez wypalenia highlights.
    var val = pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.15));
    // 1.25: AgZ wraca do kolorów, ale nie do jaskrawego pastelu.
    val = luma + 1.25 * (val - luma);
    return clamp(val, vec3<f32>(0.0), vec3<f32>(1.0));
}

/// Pełny tonemapping AgX: HDR liniowe -> liniowe do sRGB.
fn agx_tonemap(x: vec3<f32>) -> vec3<f32> {
    // `max(x, 0)`: ujemne wartości (możliwe po odejmowaniu w
    // specular occlusion) dałyby NaN w `log2`.
    var val = agx_transform() * max(x, vec3<f32>(0.0));
    val = clamp(log2(max(val, vec3<f32>(1e-10))), vec3<f32>(AGX_MIN_EV), vec3<f32>(AGX_MAX_EV));
    val = (val - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
    val = agx_contrast(val);
    val = agx_look(val);
    val = agx_transform_inv() * val;
    // Wrót do liniowych: resztę sRGB robi sprzęt przy zapisie.
    return pow(max(val, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Głębokość piksela w jednostkach świata (odległość od oka).
///
/// Bufor głębokości trzyma głębokość NORMALIZOWANĄ, która rozkłada się
/// nieliniowo: 90% zakresu liczb leży w ostatnich 10% odległości.
/// Bez liniaryzacji DoF i kontury działałyby w „przestrzeni obrazu",
/// a nie w metrach — rozmycie dalej od kamery rosłoby wykładniczo,
/// a nie liniowo.
///
/// Wzór: `z = near·far / (far - d·(far - near))`, wywiedziony
/// z odwrócenia macierzy perspektywy.
fn linear_depth(d: f32) -> f32 {
    let near = p.depth.x;
    let far = p.depth.y;
    // `d = 1.0` oznacza niebo (nic nie narysowane). Wracamy `far`,
    // żeby mgła i DoF traktowały niebo jak obiekt „w nieskończoności".
    if (d >= 1.0) {
        return far;
    }
    return (near * far) / max(far - d * (far - near), 1e-6);
}

/// Odległość piksela od oka w jednostkach świata.
fn view_depth(uv: vec2<f32>) -> f32 {
    // Próbka `textureLoad` (nie `Sample`), bo głębokość jest formatem
    // niefiltrowalnym — `textureSample` na `texture_depth_2d` wymaga
    // `sampler_comparison` i zwróciłoby wynik porównania, nie wartość.
    let dim = vec2<i32>(textureDimensions(depth_tex));
    let px = vec2<i32>(clamp(uv * vec2<f32>(dim), vec2<f32>(0.0), vec2<f32>(dim) - 1.0));
    return linear_depth(textureLoad(depth_tex, px, 0));
}

/// Normalna w przestrzeni oka + chropowatość z G-Bufera.
fn gbuffer(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(gbuf_tex, samp, uv, 0.0);
}

// --- pass 1: zostawiamy tylko jasne partie ---------------------------
// Miękki próg (próg ± k) zamiast ostrego: przy animowanym obrazie
// ostry próg migocze jak kurzlew na krawędziach poświaty.
@fragment
fn fs_bright(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src, samp, in.uv).rgb;
    let l = luma(c);
    let t = p.fx.x;
    let k = 0.3;
    let w = clamp((l - (t - k)) / (2.0 * k), 0.0, 1.0);
    return vec4<f32>(c * w, 1.0);
}

// --- pass 2/3: Gaussa 9-tapowy w jednej osi --------------------------
// Wagi [1,4,6,4,1] + środek 1.0 dają wąski, „filmowy" rozmaz zamiast
// plamistej plamy. Oś wybiera `grade.x` w tym passie (0 = pion,
// 1 = poziom) — patrz `PostFx::set_blur_axis`.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let horiz = p.grade.x > 0.5;
    let dir = select(
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), horiz
    ) / p.screen.xy;

    var sum = textureSample(src, samp, in.uv).rgb * 0.204164;
    sum = sum + textureSample(src, samp, in.uv + dir * 1.411765).rgb * 0.304005;
    sum = sum + textureSample(src, samp, in.uv - dir * 1.411765).rgb * 0.304005;
    sum = sum + textureSample(src, samp, in.uv + dir * 3.294118).rgb * 0.093913;
    sum = sum + textureSample(src, samp, in.uv - dir * 3.294118).rgb * 0.093913;
    return vec4<f32>(sum, 1.0);
}

// ======================================================================
//  SSR — Screen-Space Reflections
// ======================================================================
//
// Odbicia lustrzane policzone z tego, co jest WIDOCZNE na ekranie.
// Ray-tracing dałby pełne odbicia, ale wymagałby BVH i setek próbek
// na piksel. SSR daje większość efektu przy 20 próbkach, bo sceny
// gier są w przeważającej części zamknięte (obiekt odbijający ma
// coś, co odbić).
//
// ## Ograniczenie, które WIDAĆ na ekranie
//
// SSR nie widzi tego, co poza kadrem. Dlatego w kompozycji mieszamy
// SSR z fresnellem tak, żeby brak odbicia nie wyglądał jak ciemna
// plama, tylko jak brak połysku.

// Ile próbek na promień. Więcej = dokładniej, ale wolniej; 20 to
// kompromis, przy którym krawędź odbicia jest jeszcze gładka.
const SSR_STEPS: i32 = 20;

@fragment
fn fs_ssr(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let g = gbuffer(uv);
    let roughness = g.w;

    // Odbijamy tylko powierzchnie dostatecznie gładkie. Metal
    // o chropowatości 0.9 to dyfuzor, nie lustro — próba odbijania
    // dałaby szum, który po rozmyciu wygląda jak brud.
    if (p.ssr.x <= 0.0 || roughness > p.ssr.w) {
        return vec4<f32>(0.0);
    }

    // Wektor do oka w przestrzeni oka to `(0,0,-1)`, więc odbicie
    // lustrzane to `reflect((0,0,-1), N)`. Nie potrzebujemy macierzy
    // świata — G-Bufer trzyma normalne już w przestrzeni oka.
    let N = normalize(g.xyz);
    let R = reflect(vec3<f32>(0.0, 0.0, -1.0), N);

    let origin_depth = view_depth(uv);
    // Skala kroku: dalej obiekt, tym drobniejsze próby w UV, bo
    // obraz „ściśla się" w głębi.
    let scale = clamp(6.0 / (1.0 + origin_depth * 0.25), 0.4, 6.0);
    let step_uv = R.xy * 0.012 * scale;

    var pos = uv + step_uv * 0.5;
    var found = 0.0;
    var hit_color = vec3<f32>(0.0);
    var hit_uv = uv;
    var last_pos = uv;
    var last_depth = origin_depth;

    for (var i = 0; i < SSR_STEPS; i = i + 1) {
        last_pos = pos;
        // Krok geometryczny: 1.0, 1.35, 1.7, 2.05, ... Stała dałaby
        // albo za mało próbek na dalekie odbicia, albo za dużo
        // na bliskie.
        pos = pos + step_uv * (1.0 + f32(i) * 0.35);
        if (pos.x < 0.0 || pos.x > 1.0 || pos.y < 0.0 || pos.y > 1.0) {
            break;
        }
        last_depth = view_depth(pos);

        // Zderzenie: promień wszedł na głębokość, na której jest
        // widoczna inna powierzchnia. `ssr.z` to tolerancja: zbyt
        // mała daje dziury na stromych powierzchniach, zbyt duża —
        // odbicia „przepływają" przez cienkie obiekty.
        let diff = last_depth - origin_depth;
        let thickness = p.ssr.z;
        if (diff > 0.0 && diff < thickness) {
            // Poprawka krawędzi: trafienie jest między `last_pos`
            // (przed skokiem) a `pos` (po). Interpolacja zmniejsza
            // schodki, które zwykłe trafienie zostawia na łamaniu.
            let edge = clamp(diff / max(thickness, 1e-4), 0.0, 1.0);
            hit_uv = mix(last_pos, pos, edge);
            hit_color = textureSampleLevel(src, samp, hit_uv, 0.0).rgb;
            // Pewność: bliższe odbicie = trafniejsze. Odległe
            // odbicia są mniej wiarygodne (promień mógł przeoczyć
            // cienki obiekt), więc je przygaszamy.
            found = clamp(1.0 - f32(i) / f32(SSR_STEPS), 0.25, 1.0);
            break;
        }
        // Promień wszedł ZA daleko (przeszedł obiekt) — dalej odbicia
        // już nie będzie.
        if (diff > thickness * 3.0) {
            break;
        }
    }

    if (found <= 0.0) {
        return vec4<f32>(0.0);
    }
    // Rozmycie odbicia proporcjonalne do chropowatości: lustro
    // (0.05) ma być ostre, mat (0.4) rozlane. Osobny pass rozmycia
    // byłby droższy niż kilka próbek tutaj.
    let blur = roughness * 0.02;
    var sum = hit_color * 0.5;
    sum = sum + textureSampleLevel(src, samp, hit_uv + vec2<f32>(blur, 0.0), 0.0).rgb * 0.125;
    sum = sum + textureSampleLevel(src, samp, hit_uv - vec2<f32>(blur, 0.0), 0.0).rgb * 0.125;
    sum = sum + textureSampleLevel(src, samp, hit_uv + vec2<f32>(0.0, blur), 0.0).rgb * 0.125;
    sum = sum + textureSampleLevel(src, samp, hit_uv - vec2<f32>(0.0, blur), 0.0).rgb * 0.125;

    // Alfa niesie pewność trafienia — kompozycja używa go do
    // rozmycia przejścia z fresnellem.
    return vec4<f32>(sum, found * p.ssr.x);
}

// ======================================================================
//  God Rays — promienie słoneczne
// ======================================================================
//
// Radialne rozmycie jasnych partii obrazu W KIERUNKU SŁOŃCA. Efekt
// działa, bo jasne źródło zostawia w powietrzu rozproszony „ślad",
// który na zdjęciu widać jako smugę światła.
//
// Działa nawet bez geometrii w kadrze: wystarczy, że słońce jest
// wysoko lub poza kadrem. To najtańszy efekt atmosferyczny, jaki
// da się wyciągnąć z samego bufora koloru.

// 48 próbek — tyle potrzeba, żeby smugi nie były „przerywane".
const RAY_STEPS: i32 = 48;

@fragment
fn fs_godray(in: VsOut) -> @location(0) vec4<f32> {
    // Słońce poza kadrem nie może „świecić" z krawędzi — wtedy
    // promienie byłyby artefaktem, nie zjawiskiem.
    if (p.sun.z <= 0.0) {
        return vec4<f32>(0.0);
    }
    let uv = in.uv;
    // Wektor od piksela do słońca. Oś Y odwracamy, bo w UV rośnie
    // w dół, a `p.sun.y` podaliśmy w NDC. Bez tej negacji smugi
    // szłyby w dół zamiast ku słońcu.
    let to_sun = vec2<f32>(p.sun.x, 1.0 - p.sun.y) - uv;
    // `rays.y` rozciąga smugę: mała wartość = krótki ślad tuż przy
    // słońcu, duża = długi snop przez cały kadr.
    let step_uv = to_sun * (p.rays.y / f32(RAY_STEPS));
    var pos = uv;
    var illum = 1.0;
    var sum = vec3<f32>(0.0);

    for (var i = 0; i < RAY_STEPS; i = i + 1) {
        pos = pos + step_uv;
        if (pos.x < 0.0 || pos.x > 1.0 || pos.y < 0.0 || pos.y > 1.0) {
            break;
        }
        // Próbkujemy pełny HDR, ale przepuszczamy PRÓG: niebo jest
        // jasne od horyzontu w górę i wciągnęłoby się w smugi jako
        // jednolita szarość zamiast wiązki światła.
        let c = textureSampleLevel(src, samp, pos, 0.0).rgb;
        let bright = max(max(c.r, c.g), c.b);
        // Miękki próg: twardy dałby widoczne krawędzie smug.
        sum = sum + c * smoothstep(p.fx.x, p.fx.x * 2.0, bright) * illum;
        // Tłumienie co krok: smugi bliżej słońca są jaśniejsze,
        // co odpowiada temu, jak wygląda prawdziwy snop światła.
        illum = illum * 0.94;
    }
    // Średnia zamiast sumy: jasność nie zależy wtedy od liczby kroków,
    // więc zmiana gęstości nie przestawia ekspozycji smug.
    return vec4<f32>((sum / f32(RAY_STEPS)) * p.rays.x, 1.0);
}

// ======================================================================
//  SSAO — Screen-Space Ambient Occlusion (MSAO)
// ======================================================================
//
// AO sprawia, że obiekty przestają wisieć w powietrzu: przyciemnia
// narożniki, wnęki i miejsca kontaktu z podłogą.
//
// ## Dlaczego nie zwykły test kulowy
//
// Klasyczne SSAO (`dot(V, N) > 0 && |V| < radius`) ma w sobie błąd,
// którego nie da się wyeliminować lepszym biasem: próbka leżąca na
// **tej samej powierzchni**, co cieniujemy, mieści się w kuli i
// zostaje policzona jako przesłona. Na powierzchni zakrzywionej ku
// kamerze daje to ciemne plamy, które wyglądają jak brud.
//
// ## Co robimy inaczej (MSAO)
//
// Przenosimy podejście z WickedEngine (`msaoCS.hlsl`, pochodna
// Microsoft MiniEngine). Zamiast liczyć każdą próbkę osobno,
// **łączymy je w pary** symetryczne względem piksela:
//
//     porównujemy  `próbka @ +offset`  z  `próbka @ -offset`
//
// Obie mierzą to samo przenikanie w głąb kuli, więc ich różnica
// kasuje artefakt powierzchni, a pozostaje prawdziwa przesłona
// (inna powierzchnia po jednej stronie, brak po drugiej).
//
// Dodatkowo w przeciwieństwie do MiniEngine **nie potrzebujemy
// groupshared cache ani interleave** — u nas każdy piksel ma swój
// własny shader, więc próbki czytamy po prostu z bufora głębokości.

/// Liczba **par** próbek. Każda para = 2 odczyty głębokości, więc 6 par
/// = 12 próbek — tyle samo, ile brał poprzedni `AOAO_SAMPLES`.
///
/// 6 par to kompromis: mniej daje widoczne plamy, więcej kosztuje
/// liniowo (każda para to dwa odczyty tekstury) i przy 12 parach obraz
/// przestaje się poprawiać — zaczyna tylko ciemnieć.
const AO_PAIRS: i32 = 6;

/// Złoty kąt spiralny (2π/φ², φ = złoty przekrój).
///
/// Kierunki złote dają najlepsze pokrycie koła za daną liczbę próbek —
/// siatka kartezjańska zostawia widoczne przerwy po przekątnych.
const AO_SPIRAL: f32 = 2.39996323;

/// Głębokość piksela w surowym (nieliniowym) zapisie bufora.
///
/// Osobna od `view_depth`, bo AO porównuje głębokości między sobą
/// i potrzebuje surowej wartości, nie liniaryzowanej.
fn depth_raw(uv: vec2<f32>) -> f32 {
    let dim = vec2<i32>(textureDimensions(depth_tex));
    let px = vec2<i32>(clamp(uv * vec2<f32>(dim), vec2<f32>(0.0), vec2<f32>(dim) - 1.0));
    return textureLoad(depth_tex, px, 0);
}

/// Odległość od oka w metrach z głębokości nieliniowej.
fn linear_from_raw(d: f32) -> f32 {
    let near = p.depth.x;
    let far = p.depth.y;
    return (near * far) / max(far - d * (far - near), 1e-6);
}

/// Odtwarza pozycję w przestrzeni oka z UV i głębokości.
///
/// To jest sedno poprawnego SSAO. Wcześniejsza wersja porównywała
/// różnice głębokości w jednostkach nieliniowych, co dawało plamy —
/// bo na powierzchni skośnej do kamery (podłoga) głębokość rośnie
/// bardzo szybko i „różnica" nie miała nic wspólnego z odległością
/// w metrach.
///
/// Wzór: promień przez piksel to `(ndc.x·aspect·tan, ndc.y·tan, -1)`,
/// a pozycja to promień × liniowa głębokość.
fn view_position(uv: vec2<f32>, linear: f32) -> vec3<f32> {
    // NDC: `+1` to góra, tekstura ma `v` rosnące w dół.
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let tan_v = p.proj.x;
    let dir = vec3<f32>(ndc.x * p.proj.y * tan_v, ndc.y * tan_v, -1.0);
    return dir * linear;
}

@fragment
fn fs_ssao(in: VsOut) -> @location(0) vec4<f32> {
    // `1.0` = „nic nie zasłania", wartość neutralna dla tła.
    if (p.ssao.x <= 0.0) {
        return vec4<f32>(1.0);
    }
    let uv = in.uv;
    let d = depth_raw(uv);
    // Niebo (`d = 1.0`) nie ma normalnej ani otoczenia, które mogłoby
    // je zaciemnić. Bez tego wyjścia niebo dostałoby czarne plamy
    // tam, gdzie próbka trafi w horyzont.
    if (d >= 1.0) {
        return vec4<f32>(1.0);
    }

    let g = gbuffer(uv);
    // Normalna w przestrzeni oka. Piksel z niczym w G-Buferze ma
    // zerową normalną — dzielenie dałoby NaN, więc pomijamy go.
    let n_len = length(g.xyz);
    if (n_len < 1e-4) {
        return vec4<f32>(1.0);
    }
    let N = g.xyz / n_len;

    let linear = linear_from_raw(d);
    // Uwaga: od wersji z testem par **nie przesuwamy środka kuli
    // wzdłuż normalnej**. W MSAO odrzucanie par robi dokładnie to, co
    // robił dawny `ssao_bias` — a robi to lepiej, bo nie przesuwa całej
    // kuli i nie zniekształca AO na cienkich obiektach.
    //
    // Pole `ssao_bias` zostało w uniformie, bo `PostSettings` jest
    // publicznym API i demo mogą je ustawiać; shader używa go teraz
    // jako dolnego limitu `inv_thickness` (patrz niżej), więc wartość
    // nadal ma wpływ na wynik.

    // Promień w METACH. Próbki rozłożone w PLASZCZYŹNIE obrazu
    // mają różną odległość, więc ich przesunięcie w UV musi rosnąć
    // z odległością — inaczej dalekie próbki trafiają w promień
    // kilku centymetrów zamiast kilkudziesięciu.
    let radius = p.ssao.y;
    // Przesunięcie w UV na jeden metr w płaszczyźnie prostopadłej do
    // kierunku patrzenia. Wynika wprost z geometrii rzutu: obiekt
    // oddalony o `linear` metrów przesuwa się o
    // `metr / (tan(FOV) · linear)` wektorów UV. Stąd `linear` w
    // mianowniku — wzór `linear / far` działa przy głębokości
    // nieliniowej i zaciemniałby cały horyzont.
    let uv_per_meter = vec2<f32>(
        1.0 / (p.proj.x * p.proj.y * linear),
        1.0 / (p.proj.x * linear),
    );

    // Wzór MSAO operuje na `1 / głębokość` zamiast na samej głębokości,
    // bo w tej skali odległość jest liniowa. Bez tego próbki blisko
    // kamery miałyby nieproporcjonalnie duży wpływ na wynik i AO
    // byłoby silne tylko na bliskich powierzchniach.
    //
    // `bias` wchodzi tu jako **margines grubości**: zwiększa tyle, o ile
    // kula jest „grubsza" niż promień, przez co próbki tuż przy
    // powierzchni wypadają poza kulę. To dokładnie jego dawna rola
    // („nie próbkuj samej powierzchni") — tylko wyrażona w skali
    // odwróconej głębokości, a nie jako przesunięcie środka.
    let inv_thickness = 1.0 / max(radius + p.ssao.z, 1e-4);
    let inv_range = inv_thickness * (1.0 / max(linear, 1e-4));
    // Referencyjna głębokość „przodu kuli" w tej samej skali.
    let front_depth = inv_thickness - 0.5;

    var occlusion = 0.0;
    var pairs_used = 0.0;
    for (var i = 0; i < AO_PAIRS; i = i + 1) {
        let fi = f32(i);
        // Spirala: kąt rośnie liniarnie z `i`, promień jak `sqrt`,
        // żeby próbki równomiernie wypełniły koło.
        let angle = fi * AO_SPIRAL;
        let dist = radius * sqrt((fi + 0.5) / f32(AO_PAIRS));
        let offset = vec2<f32>(cos(angle), sin(angle)) * dist * uv_per_meter;
        // PARA: dwie próbki symetryczne względem piksela. To one
        // odróżniają MSAO od zwykłego SSAO — patrz komentarz sekcji.
        let uv_a = uv + offset;
        let uv_b = uv - offset;
        // Para wychodząca poza kadr: nie mamy tam danych, więc całą
        // parę pomijamy. Liczenie jej jako „otwartej" dawałoby ciemne
        // krawędzie kadru.
        let in_frame = uv_a.x > 0.0 && uv_a.x < 1.0 && uv_a.y > 0.0 && uv_a.y < 1.0
            && uv_b.x > 0.0 && uv_b.x < 1.0 && uv_b.y > 0.0 && uv_b.y < 1.0;
        if (!in_frame) {
            continue;
        }
        let sd_a = depth_raw(uv_a);
        let sd_b = depth_raw(uv_b);
        // Niebo (`d = 1.0`) nie zasłania niczego. Pomijamy taką parę
        // w całości — inaczej krawędź nieba ciemniałaby okolice.
        if (sd_a >= 1.0 || sd_b >= 1.0) {
            continue;
        }
        pairs_used = pairs_used + 1.0;

        // „Disocclusion": jak głęboko próbka wnika w kulę.
        // < 0 = pełne przesłony, > 1 = wcale (za kulą).
        let dis1 = (1.0 / linear_from_raw(sd_a)) * inv_range - front_depth;
        let dis2 = (1.0 / linear_from_raw(sd_b)) * inv_range - front_depth;

        // Pseudo-disocclusion: ograniczenie disocclusion od dołu.
        // To sedno metody — jeśli obie próbki mają tę samą wartość
        // (obie na powierzchni albo obie daleko), ograniczenie je
        // wyrównuje i para daje ~0. Jeśli jedna zasłania, a druga nie,
        // różnica zostaje. `rays.z` to `xRejectFadeoff` z MSAO.
        let p1 = clamp(p.rays.z * dis1, 0.0, 1.0);
        let p2 = clamp(p.rays.z * dis2, 0.0, 1.0);

        // Test pary z MiniEngine: dwie wzajemnie ograniczone wartości,
        // minus iloczyn (usuwa podwójne policzenie tego samego cienia
        // w obu próbkach). Wynik ograniczamy do 0..1, bo suma z ujemnymi
        // `dis` mogłaby wyjść poza zakres i odwrócić sens AO.
        occlusion = occlusion + clamp(
            clamp(dis1, p2, 1.0) + clamp(dis2, p1, 1.0) - p1 * p2,
            0.0,
            1.0
        );
    }

    // Średnia, a nie suma — inaczej zmiana liczby par zmieniałaby
    // jasność AO. `pairs_used` bywa 0 na krawędzi kadru; wtedy
    // zwracamy 1.0 (brak cienia), żeby nie dzielić przez zero.
    if (pairs_used < 0.5) {
        return vec4<f32>(1.0);
    }
    let ao = clamp(1.0 - occlusion / pairs_used, 0.0, 1.0);
    // `intensity` pogłębia narożniki. Bez niego AO jest zbyt płytkie
    // i ledwo widać.
    let ao_pow = pow(ao, p.ssao.w);
    return vec4<f32>(ao_pow, 0.0, 0.0, 1.0);
}

/// Rozmycie AO: 4×4 box, 16 próbek.
///
/// SSAO z 12 próbek jest silnie szumowne, a szum w zacienieniu
/// wygląda jak brud na obiektywie. Rozmycie w dół jest obowiązkowe.
@fragment
fn fs_ao_blur(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / p.screen.xy;
    var sum = 0.0;
    for (var y: i32 = -2; y <= 1; y = y + 1) {
        for (var x: i32 = -2; x <= 1; x = x + 1) {
            sum = sum + textureSample(ao_tex, samp, in.uv + vec2<f32>(f32(x), f32(y)) * texel).r;
        }
    }
    // Zwracamy `vec4`, bo takiego typu oczekuje deklaracja
    // `@location(0)`. Cel to `R8Unorm`, więc wgpu zapisze tylko
    // kanał R — pozostałe są ignorowane przez format.
    return vec4<f32>(sum / 16.0, 0.0, 0.0, 1.0);
}

// --- pass 4: kompozycja ----------------------------------------------
@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let texel = 1.0 / p.screen.xy;
    let centered = uv - vec2<f32>(0.5);

    // ==================================================================
    //  1. Depth of Field — rozmycie głębi pola
    // ==================================================================
    //
    // Koło nieostrości (CoC) z modelu fizycznego: obiekty w odległości
    // ogniskowej są ostre, a poniżej i powyżej niej rozmywają się
    // proporcjonalnie do ODWROTNOŚCI odległości — jak w prawdziwym
    // obiektywie. Zwykły `abs(z - focus)` dawałby stałe rozmycie
    // w całym kadrze i niczego by nie uczył widza.
    let d = view_depth(uv);
    let focus = max(p.scatter.w, 0.001);
    // Reciprocal — podstawa modelu cienkiej soczewki.
    let coc = (1.0 / max(d, 0.001) - 1.0 / focus) * p.lens.x;
    // Promień w pikselach, z limitem: zbyt duże CoC zamienia obraz
    // w plamę, a `max_blur` chroni przed tym.
    let radius = clamp(abs(coc) * p.screen.x, 0.0, p.lens.y);

    var col: vec3<f32>;
    if (radius < 0.5) {
        // W odległości ogniskowej (albo gdy DoF wyłączone) — jeden
        // odczyt. Osobna gałąź, bo pętla po 12 próbkach dla piksela
        // o promieniu 0 to czysta strata.
        col = textureSample(src, samp, uv).rgb;
    } else {
        // Złoty spiral: 12 próbek rozmieszczonych spiralą (Fibonacci)
        // daje dużo gładniejszy rozmaz niż 12 próbek w siatce, przy
        // tej samej liczbie odczytów. Spiralna próba NIE jest
        // „okrągła" — stąd nazwa „bokeh" w DoF.
        var acc = textureSample(src, samp, uv).rgb;
        var total = 1.0;
        // Złoty kąt w radianach: 2π/φ², φ = złoty przekrój.
        let golden = 2.39996323;
        for (var i = 0; i < 12; i = i + 1) {
            let fi = f32(i);
            let angle = fi * golden;
            // Pierścień: `sqrt` rozkłada próby równomiernie po
            // powierzchni koła, a nie po jego średnicy.
            let r = sqrt((fi + 0.5) / 12.0) * radius * texel;
            let offset = vec2<f32>(cos(angle), sin(angle)) * r;
            let s = textureSample(src, samp, uv + offset).rgb;
            acc = acc + s;
            total = total + 1.0;
        }
        col = acc / total;
    }

    // ==================================================================
    //  2. Subsurface Scattering — podpowierzchniowe rozpraszanie
    // ==================================================================
    //
    // Światło przechodzące przez cienkie fragmenty (uszy, palce,
    // krawędź nosa) rozlewa się na sąsiednie piksele i przesuwa
    // ciepłą poświatę poza kontur obiektu. Robimy to tu, na gotowym
    // obrazie, próbkując sąsiadów w promieniu zależnym od odwróconej
    // normalnej — dzięki temu poświat wychodzi NA zewnątrz obrysu,
    // czyli dokładnie tam, gdzie SSS jest widoczne.
    if (p.scatter.x > 0.0) {
        let g = gbuffer(uv);
        // Gdzie „tył" normalnej — tam będzie wychodzić poświat.
        let back = clamp(-g.z, 0.0, 1.0);
        // Promień w UV: krótki, bo SSS to zjawisko lokalne.
        // `texel` jest wektorem, więc `sr` też — stąd `.x` w warunku.
        let sr = p.scatter.y * texel;
        if (back > 0.01 && sr.x > 0.0) {
            // Kierunek „przez obiekt": przeciwny do normalnej.
            let dir = normalize(g.xyz).xy * sr;
            var bleed = vec3<f32>(0.0);
            bleed = bleed + textureSample(src, samp, uv - dir * 1.0).rgb;
            bleed = bleed + textureSample(src, samp, uv + dir * 1.0).rgb;
            bleed = bleed + textureSample(src, samp, uv - dir * 0.5 + vec2<f32>(0.0, sr.y)).rgb;
            bleed = bleed + textureSample(src, samp, uv + dir * 0.5 - vec2<f32>(0.0, sr.y)).rgb;
            bleed = bleed * 0.25;
            // Ciepły przesunięcie: krew pochłania fale zielone,
            // więc poświat wychodząca spod skóry jest różowa.
            let warm = vec3<f32>(1.15, 0.62, 0.48);
            // Mieszamy tylko tam, gdzie sąsiad jest jasny — inaczej
            // ciemna krawędź obiektu „wyciekłaby" na tło.
            col = mix(col, col + bleed * warm, p.scatter.x * back);
        }
    }

    // ==================================================================
    //  3. Screen-Space Reflections
    // ==================================================================
    //
    // Mieszamy odbicie z fresnellem, nie dodajemy go wprost. Fresnel
    // mówi „ile światła odbija się pod tym kątem" — pod kątem 90°
    // (krawędź obiektu) niemal wszystko, prostopadle do widzenia —
    // prawie nic. Bez niego odbicie byłoby równomiernie nałożone na
    // całą powierzchnię i wyglądałoby jak lakier.
    if (p.ssr.x > 0.0) {
        let s = textureSampleLevel(ssr_tex, samp, uv, 0.0);
        let g = gbuffer(uv);
        let N = normalize(g.xyz);
        // Wektor do oka w przestrzeni oka: stały `(0,0,-1)`.
        let ndotv = clamp(dot(N, vec3<f32>(0.0, 0.0, -1.0)), 0.0, 1.0);
        // Schlick: F(0) = 0.04 (typowe F0 dielektryka).
        let fresnel = 0.04 + 0.96 * pow(1.0 - ndotv, 5.0);
        // Alfa z `fs_ssr` to pewność trafienia — mnożymy, żeby
        // nieudane odbicia nie dodawały szumu.
        let amount = fresnel * s.a;
        col = mix(col, s.rgb, clamp(amount, 0.0, 1.0));
    }

    // ==================================================================
    //  4. Selective outlining — kontury z głębokości i normalnych
    // ==================================================================
    //
    // Klasyczny gruby, czarny kontur (inverted hull) psuje tu cały
    // efekt: otacza obiekt twardą, geometryczną ramą, która wygląda
    // jak naklejka. Zamiast tego liczymy krawędź z RÓŻNIC sąsiednich
    // pikseli głębokości i normalnych.
    //
    // Różnica jest tu kluczowa: krawędź obiektu to nie tylko skok
    // głębokości, ale też zwrot normalnej. Na płaskiej ścianie
    // głębokość jest różna, ale normalna stała — i kontur znika,
    // zamiast obrysowywać każdą płaszczyznę.
    //
    // Kontur ZANIKA w jasnych miejscach (`outline.w`) — dlatego
    // wygląda dynamicznie: na słońcu go nie ma, w cieniu jest.
    if (p.outline.x > 0.0) {
        let t = p.outline.y * texel;
        // Cztery sąsiadzie w krzyżu. Krzyż, nie pełne 8 — osiem
        // próbek daje lepszą gładkość, ale kontur i tak ma być
        // 1-2 px szerokości, więc różnica jest niewidoczna.
        let dl = view_depth(uv - vec2<f32>(t.x, 0.0));
        let dr = view_depth(uv + vec2<f32>(t.x, 0.0));
        let du = view_depth(uv - vec2<f32>(0.0, t.y));
        let dd = view_depth(uv + vec2<f32>(0.0, t.y));

        // Skok głębokości względem ODLĘGŁOŚCI, nie absolutny.
        // Różnica bezwzględna dawałaby kontur na każdej płaszczyźnie
        // ustawionej pod kątem do kamery — czyli prawie wszędzie.
        let center = view_depth(uv);
        let depth_edge = (abs(dl - dr) + abs(du - dd)) / max(center, 1.0);

        // Zmiana normalnej — łapie krawędzie, gdzie głębokość jest
        // ciągła (zaokrąglona krawędź obiektu, styk dwóch ścian).
        let nl = gbuffer(uv - vec2<f32>(t.x, 0.0)).xyz;
        let nr = gbuffer(uv + vec2<f32>(t.x, 0.0)).xyz;
        let nu = gbuffer(uv - vec2<f32>(0.0, t.y)).xyz;
        let nd = gbuffer(uv + vec2<f32>(0.0, t.y)).xyz;
        let normal_edge = (length(nl - nr) + length(nu - nd)) * 0.5;

        // Łączymy oba sygnały. `outline.z` to próg, który odsiewa
        // szum z dokładności głębokości (Depth32Float ma ~7 cyfr, ale
        // po liniaryzacji i tak zostaje resztkowy błąd).
        var edge = max(depth_edge, normal_edge);
        let line = smoothstep(p.outline.z, p.outline.z * 3.0, edge);

        // Zanikanie w świetle: tam, gdzie piksel jest jasny, kontur
        // się rozpuszcza. To ten sam mechanizm, który w animacji
        // daje wrażenie „linii rysowanej tuszem, która znika na
        // słońcu".
        let bright = luma(col);
        let fade = 1.0 - smoothstep(0.4, 1.6, bright) * p.outline.w;

        let a = clamp(line * fade * p.outline.x, 0.0, 1.0);
        // Kontur nie jest czarny: bierze kolor z otoczenia i przygasza
        // go, zamiast malować wewnętrzność piksela. Dlatego wygląda
        // jak obrys rysowany na obrazie, a nie jak dziura w nim.
        let ink = col * 0.35;
        col = mix(col, ink, a);
    }

    // ==================================================================
    //  5. Anamorphic bloom — poziome smugi
    // ==================================================================
    //
    // Obiektywy filmowe mają przednią soczewkę o kształcie elipsy
    // albo specjalny element rozpraszający, przez który źródło światła
    // zostawia POZIOMĄ smugę zamiast okrągłej plamy. To jeden z
    // najbardziej rozpoznawalnych elementów kina akcji.
    //
    // Robimy go jako dodatkowe, szerokie rozmycie poziome tej samej
    // przepuszczonej mapy. Osobny bloom kosztowałby drugie tyle próbek.
    if (p.lens.z > 0.0) {
        // Szeroki, niski kernel: 7 tapów rozciągniętych na ~18 px.
        // Jakość jest drukciarska, nie filmowa — ale przy anamorficznej
        // poświaci nikt nie patrzy na krawędź smugi.
        var streak = vec3<f32>(0.0);
        let sw = vec2<f32>(18.0 * texel.x, 0.0);
        streak = streak + textureSample(bloom_tex, samp, uv - sw).rgb;
        streak = streak + textureSample(bloom_tex, samp, uv - sw * 0.66).rgb;
        streak = streak + textureSample(bloom_tex, samp, uv - sw * 0.33).rgb;
        streak = streak + textureSample(bloom_tex, samp, uv).rgb;
        streak = streak + textureSample(bloom_tex, samp, uv + sw * 0.33).rgb;
        streak = streak + textureSample(bloom_tex, samp, uv + sw * 0.66).rgb;
        streak = streak + textureSample(bloom_tex, samp, uv + sw).rgb;
        streak = streak / 7.0;
        // Smuga jest zawsze odchłodzona (niebieska) — to cecha
        // konstrukcji obiektywu, a nie ustawienie filmu.
        col = col + streak * vec3<f32>(0.55, 0.78, 1.25) * p.lens.z;
    }

    // ==================================================================
    //  6. Bloom (zwykły, okrągły)
    // ==================================================================
    col = col + textureSample(bloom_tex, samp, uv).rgb * p.grade.y;

    // ==================================================================
    //  7. God rays — promienie słoneczne
    // ==================================================================
    //
    // Dokładamy PO tonemappingu: to zjawisko optyczne w obiektywie,
    // a nie światło sceny, więc powinno mieć własny zakres jasności
    // niezależny od ekspozycji.
    if (p.sun.z > 0.0) {
        col = col + textureSampleLevel(rays_tex, samp, uv, 0.0).rgb;
    }

    // ==================================================================
    //  8. Flara obiektywu (lens flare)
    // ==================================================================
    //
    // Ghosty: wewnętrzne odbicia, gdy światło odbija się od przepon
    // kilka razy. Rysujemy je wzdłuż prostej słońce–środek kadru —
    // to ich charakterystyczne położenie.
    if (p.lens.w > 0.0 && p.sun.z > 0.0) {
        let sun_uv = vec2<f32>(p.sun.x, 1.0 - p.sun.y);
        let delta = (vec2<f32>(0.5) - sun_uv) / 4.0;
        var flare = vec3<f32>(0.0);
        // Cztery ghosty o malejącej sile. Kolory pochodzą z powłok
        // antyrefleksyjnych, więc są „dziwne" barwy dopełniające.
        for (var i = 0; i < 4; i = i + 1) {
            let t = f32(i) * 0.7 + 0.35;
            let pos = sun_uv + delta * t;
            let d = length(uv - pos);
            let r = 0.012 + f32(i) * 0.004;
            // `smoothstep` z dwoma promieniami daje miękki brzeg kuli.
            let g = smoothstep(r, r * 0.35, d);
            let hue = vec3<f32>(
                0.5 + 0.5 * cos(f32(i) * 2.1),
                0.5 + 0.5 * cos(f32(i) * 2.1 + 2.1),
                0.5 + 0.5 * cos(f32(i) * 2.1 + 4.2),
            );
            flare = flare + g * hue * (0.5 / (t * t));
        }
        // Aureola: miękka poświata tuż przy tarczy słońca.
        //
        // Promień 0.28 UV był za duży — to 28% szerokości kadru, więc
        // poświata stawała się wielkim, twardym krążkiem żółtego koloru
        // widać na niebie zamiast aureolą przy tarczy. Trzymamy ją na
        // 0.035 (kilka procent kadru) i dodajemy potęgę, żeby spadka
        // była miękka, a nie liniowa.
        let halo = pow(1.0 - smoothstep(0.0, 0.035, length(uv - sun_uv)), 2.0);
        flare = flare + halo * vec3<f32>(1.0, 0.9, 0.75) * 0.35;
        col = col + flare * p.lens.w;
    }

    // ==================================================================
    //  8b. SSAO — cieniowanie kontaktowe
    // ==================================================================
    //
    // AO przyciemnia TYLKO to, co pochodzi z otoczenia. Nie wolno
    // mnożyć nim całego obrazu, bo zaciemniłoby też odbicie słońca
    // na wypolerowanej podłodze i światła emisyjne — a one od
    // otoczenia nie zależą.
    //
    // Problem: w tym rendererze diffuse i specular są już zsumowane
    // w jednym HDR, więc po fakcie nie da się ich rozdzielić. Stąd
    // rozwiązanie kompromisowe: chronimy partie, które są ZDEFINOWANIE
    // za jasne, żeby zaciennić — emisja i bezpośrednie światło
    // od słońca. Robimy to przez wykrycie „jasności względnej":
    // jeśli piksel jest jaśniejszy niż typowa litna powierzchnia,
    // uznajemy go za światło i nie ruszamy.
    if (p.ssao.x > 0.0) {
        let ao = textureSample(ao_tex, samp, uv).r;
        // Próg jasności: powyżej tej wartości (w liniowym HDR) piksel
        // jest uznany za źródło światła. 1.5 to wartość, przy której
        // zwykła,oświetlona powierzchnia jest już poza progiem, a
        // neon i odbicie tarczy jeszcze nie.
        let bright = luma(col);
        let is_light = smoothstep(1.0, 1.5, bright);
        // Emitery są bardzo jasne i jednokolorowe; dodatkowo nie
        // cieniujemy niczego, co jest jasniejsze od progu.
        let ao_factor = mix(ao, 1.0, is_light);
        // Siła z ustawień, a nie z uniformu passu — tu liczymy
        // tylko sam czynnik, resztę robi `mix` niżej.
        let shaded = mix(vec3<f32>(1.0), vec3<f32>(clamp(ao_factor, 0.0, 1.0)), p.ssao.x);
        // Tonemapping jest dalej, więc operujemy na wartościach
        // liniowych — mnożenie jest tu poprawne.
        col = col * shaded;
    }

    // ==================================================================
    //  9. Ekspozycja + tonemapping
    // ==================================================================
    col = agx_tonemap(col * p.grade.x);

    // ==================================================================
    //  10. Color grading — podział tonów
    // ==================================================================
    //
    // Zamiast LUT podnosimy kanały osobno dla cieni i dla świateł.
    // To daje „filmową" paletę: chłodne, niebieskie cienie i ciepłe,
    // pastelowe światła. Tablica LUT byłaby dokładniejsza, ale
    // wymagałaby tekstury 3D z interp. Podział tonów daje 90% efektu
    // przy zerowej pamięci i jednej operacji na piksel.
    if (p.grade2.z > 0.0) {
        let l = luma(col);
        // Waga cieni: 1 dla czerni, 0 dla bieli.
        let shadow_w = 1.0 - smoothstep(0.0, 0.5, l);
        // Waga świateł: odwrotnie.
        let high_w = smoothstep(0.5, 1.0, l);
        // Podbicie kanałów: >1 rozjaśnia kanał, <1 przyciemnia.
        // Wektor `vec3(1.0)` to neutralny mnożnik.
        let shadow_tint = vec3<f32>(0.92, 0.99, 1.14);
        let high_tint = vec3<f32>(1.10, 1.04, 0.94);
        col = col * mix(vec3<f32>(1.0), shadow_tint, shadow_w * p.grade2.z);
        col = col * mix(vec3<f32>(1.0), high_tint, high_w * p.grade2.z);
    }

    // --- aberracja chromatyczna: kanały R i B z radialnym przesunięciem
    // Prawie niewidoczna w centrum, mocna na krawędziach — tak działa
    // prawdziwy obiektyw, i o to chodzi.
    if (p.grade.w > 0.0) {
        let ca = p.grade.w * dot(centered, centered);
        let cr = textureSample(src, samp, uv + centered * ca).r;
        let cb = textureSample(src, samp, uv - centered * ca).b;
        col = vec3<f32>(mix(col.r, cr, 0.8), col.g, mix(col.b, cb, 0.8));
    }

    // --- saturacja i kontrast wokół 0.5
    col = mix(vec3<f32>(luma(col)), col, p.fx.z);
    col = clamp((col - 0.5) * p.fx.w + 0.5, vec3<f32>(0.0), vec3<f32>(1.0));

    // --- winieta: przyciemnia narożniki i kieruje wzrok do centrum
    let vig = 1.0 - p.grade.z * smoothstep(0.30, 1.05, length(centered) * 1.45);
    col = col * vig;

    // --- ziarno filmowe: łamie gradienty i maskuje banding w niebie
    let g = fract(sin(dot(uv * p.screen.xy + p.screen.z, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    col = col + (g - 0.5) * p.fx.y;

    // UWAGA: zwracamy wartości LINIOWE. Format powierzchni to
    // `...Srgb`, więc sprzęt wykonuje konwersję do sRGB sam. Gdybyśmy
    // zakodowali ją tutaj, obraz byłby prześwietlony (tzw. double gamma).
    return vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
