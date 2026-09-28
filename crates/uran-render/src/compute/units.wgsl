// Rysowanie jednostek symulowanych na GPU.
//
// Kluczowa rzecz: pozycja nie przychodzi z CPU. Shader wierzchołkowy czyta
// `units[instance_index]` i dopiero tam skaluje do świata — dlatego koszt
// CPU na jednostkę jest zerowy, a dropnięcie liczby jednostek z 100k do 0
// nie kosztuje ani jednego bajta transferu.

struct Globals {
    view_proj: mat4x4<f32>,
}

struct Unit {
    position: vec2<f32>,
    velocity: vec2<f32>,
    target: vec2<f32>,
    health: f32,
    cooldown: f32,
    flags: u32,
    seed: u32,
    _pad: vec2<f32>,
}

struct Style {
    // x: promień jednostki, y: grubość obwódki, z: mnożnik alfy
    radius: f32,
    edge: f32,
    alpha: f32,
    _pad: f32,
    // kolory drużyn w liniowym świetle (4 x vec4 = 64 B)
    colors: array<vec4<f32>, 4>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> style: Style;
@group(2) @binding(0) var<storage, read> units: array<Unit>;

struct VSOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
}

const ALIVE: u32 = 1u;
const TEAM_BIT: u32 = 2u;
const ATTACKED: u32 = 4u;

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
) -> VSOut {
    var out: VSOut;

    let u = units[ii];
    let alive = (u.flags & ALIVE) != 0u;

    // martwą jednostkę zwijamy do zdegenerowanego trójkąta (poza NDC),
    // dzięki czemu GPU odrzuca ją przy rasteryzacji — bez branchy na CPU
    if (!alive) {
        out.clip_position = vec4<f32>(0.0, 0.0, -10.0, 1.0);
        out.local = vec2<f32>(0.0);
        out.color = vec4<f32>(0.0);
        return out;
    }

    // jednostka (-0.5..0.5) z 6 indeksów
    var corner = vec2<f32>(-0.5, 0.5);
    if (vi == 1u || vi == 4u) { corner = vec2<f32>(0.5, 0.5); }
    if (vi == 2u || vi == 3u) { corner = vec2<f32>(0.5, -0.5); }
    if (vi == 5u) { corner = vec2<f32>(-0.5, -0.5); }

    // ziarno modyfikuje rozmiar i jasność — armia nie jest jednolicie płaska
    let jitter = hash01(u.seed);
    let radius = style.radius * (0.75 + 0.5 * jitter);
    // lekki „oddech” formy przy strzale
    let flash = select(1.0, 1.6, (u.flags & ATTACKED) != 0u);

    let world = u.position + corner * radius * 2.0 * flash;
    out.clip_position = globals.view_proj * vec4<f32>(world, 1.0);
    out.local = corner * 2.0;

    let team = (u.flags & TEAM_BIT) != 0u;
    let base = style.colors[select(0, 1, team)];
    // zdrowie jako przyciemnienie: ranny żołnierz jest ciemniejszy
    let hp = clamp(u.health / 100.0, 0.25, 1.0);
    // kierunek ruchu daje delikatne rozjaśnienie „przodu" jednostki
    let vel_len = length(u.velocity);
    let facing = clamp(vel_len / 120.0, 0.0, 1.0);
    var c = base;
    if (team) {
        c = mix(base, vec4<f32>(1.0, 0.95, 0.85, base.a), facing * 0.35);
    } else {
        c = mix(base, vec4<f32>(0.75, 0.9, 1.0, base.a), facing * 0.35);
    }
    out.color = vec4<f32>(c.rgb * hp, c.a * style.alpha);
    return out;
}

/// Deterministyczny szum [0,1) — musi być zgodny z `sim.wgsl`.
fn hash01(v: u32) -> f32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x & 0x00ffffffu) / 16777215.0;
}

/// Kółko z wygładzonym brzegiem (SDF) — 100k sprite'ów bez tekstury.
@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {
    let d = length(in.local);
    if (d > 1.0) { discard; }
    let aa = fwidth(d) * 1.5;
    let mask = 1.0 - smoothstep(1.0 - aa, 1.0, d);
    var c = in.color;
    c.a = c.a * mask;
    return c;
}