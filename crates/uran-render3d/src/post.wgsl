// Post-processing: AA, bloom, tonemapping ACES, aberracja, winieta, ziarno.
//
// Wejście jest LINIOWE i HDR (rgba16float), wyjście to powierzchnia gry.
// Tonemapping w tej dwojstce jest konieczny — bez niego wartości powyżej
// 1.0 obcinałyby się w 8 bitach i obraz był płaski.

struct Params {
    // x = ekspozycja, y = siła bloom, z = winieta, w = aberracja
    grade: vec4<f32>,
    // x = próg bloom, y = ziarno, z = saturacja, w = kontrast
    fx: vec4<f32>,
    // x,y = rozmiar kadru, z = czas, w = siła AA
    screen: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

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

// --- ACES (Narkowicz) --------------------------------------------------
// Krzywa kompresuje jasne partie do 0..1 z zachowaniem nasycenia barw;
// zwykły `1 - exp(-x)` daje blade, przeterminowane kolory.
fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
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

// --- pass 4: kompozycja ----------------------------------------------
@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let texel = 1.0 / p.screen.xy;

    // --- AA krawędzi (FXAA w uproszczeniu)
    //
    // Scena renderujemy bez MSAA, więc krawędzie brył i pagórków
    // „zębią". Zamiast pełnego FXAA bierzemy 4 próby po przekątnych
    // i mieszamy TYLKO tam, gdzie sąsiadujące luminancje się różnią.
    // Na płaskim terenie różnica jest zerowa, więc obraz zostaje
    // ostry; na krawędziach zamiast zębów dostajemy miękkie przejście.
    let c0 = textureSample(src, samp, uv).rgb;
    var col = c0;
    if (p.screen.w > 0.0) {
        let sn = textureSample(src, samp, uv + vec2<f32>(-1.0, -1.0) * texel).rgb;
        let ss = textureSample(src, samp, uv + vec2<f32>(1.0, 1.0) * texel).rgb;
        let se = textureSample(src, samp, uv + vec2<f32>(1.0, -1.0) * texel).rgb;
        let sw = textureSample(src, samp, uv + vec2<f32>(-1.0, 1.0) * texel).rgb;
        let l0 = luma(c0);
        let lmin = min(l0, min(min(luma(sn), luma(ss)), min(luma(se), luma(sw))));
        let lmax = max(l0, max(max(luma(sn), luma(ss)), max(luma(se), luma(sw))));
        let contrast = lmax - lmin;
        // próg zależny od jasności: w cieniu artefakty są mniej widoczne
        if (contrast > 0.08 * lmax + 0.01) {
            let w = clamp(contrast * 2.0, 0.0, 1.0) * p.screen.w;
            col = mix(c0, (sn + ss + se + sw) * 0.25, w);
        }
    }

    // --- aberracja chromatyczna: kanały R i B z radialnym przesunięciem
    // Prawie niewidoczna w centrum, mocna na krawędziach — tak działa
    // prawdziwy obiektyw, i o to chodzi.
    let centered = uv - vec2<f32>(0.5);
    if (p.grade.w > 0.0) {
        let ca = p.grade.w * dot(centered, centered);
        let cr = textureSample(src, samp, uv + centered * ca).r;
        let cb = textureSample(src, samp, uv - centered * ca).b;
        col = vec3<f32>(mix(col.r, cr, 0.8), col.g, mix(col.b, cb, 0.8));
    }

    // --- bloom
    col = col + textureSample(bloom_tex, samp, uv).rgb * p.grade.y;

    // --- ekspozycja + tonemapping
    col = aces(col * p.grade.x);

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
