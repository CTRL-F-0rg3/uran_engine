// Rysowanie 100k jednostek: jedna instancja = jeden sprite, pozycja
// czytana wprost z bufora storage w shaderze wierzchołkowym. CPU nie
// buduje listy wierzchołków ani nie tka sprite'ów.
//
// Layout bind grup musi się zgadzać z `build_draw_pipeline`:
//   @group(0) @binding(0) globals  — macierz świata -> NDC (jak w shader.wgsl)
//   @group(1) @binding(0) style    — UnitStyle (promień, kolory drużyn)
//   @group(2) @binding(0) units    — pozycje, tylko do odczytu

struct Globals {
    view_proj: mat4x4<f32>,
}

struct UnitStyle {
    radius: f32,
    edge: f32,
    alpha: f32,
    _pad: f32,
    colors: array<vec4<f32>, 4>,
}

struct Unit {
    position: vec2<f32>,
    velocity: vec2<f32>,
    goal: vec2<f32>,
    health: f32,
    cooldown: f32,
    flags: u32,
    seed: u32,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> style: UnitStyle;
@group(2) @binding(0) var<storage, read> units: array<Unit>;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) tint: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
) -> VsOut {
    // dwa trójkąty bez index buffera
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0),
    );
    let corner = corners[vi];
    let u = units[ii];

    var out: VsOut;
    let alive = (u.flags & 1u) != 0u;
    if (!alive) {
        // martwą jednostkę degenerujemy: współrzędne poza NDC + skasowany
        // fragment. Liczba instancji pozostaje stała, a GPU nie rasteryzuje.
        out.clip_pos = vec4<f32>(0.0, 0.0, 0.5, 1.0);
        out.local = corner;
        out.tint = vec4<f32>(0.0);
        return out;
    }

    // puls po strzale — lekki rozbłysk, żeby było widać akcję
    let hot = select(0.0, 1.0, (u.flags & 4u) != 0u);
    let r = style.radius * (1.0 + hot * 0.6);
    let world = u.position + corner * r;
    let ndc = globals.view_proj * vec4<f32>(world, 0.0, 1.0);

    // drużyna: bit 1 = przeciwnik, inaczej gracz
    let enemy = (u.flags & 2u) != 0u;
    let team = select(0, 1, enemy);
    let rgb = style.colors[team].rgb;
    // lekki wariacjas jasności po ziarnie, żeby armia nie była płaska
    let shade = 0.82 + 0.18 * fract(f32(u.seed & 255u) * 0.0157);
    // zdrowie: bliżej śmierci ciemniej
    let life = clamp(u.health / 100.0, 0.15, 1.0);

    out.clip_pos = ndc;
    out.local = corner;
    out.tint = vec4<f32>(rgb * shade * life * (1.0 + hot), style.alpha);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // miękki okrągły sprite zamiast kwadratu
    let d = dot(in.local, in.local);
    if (d > 1.0) { discard; }
    let edge = smoothstep(1.0, 0.5, d);
    return vec4<f32>(in.tint.rgb, in.tint.a * edge);
}