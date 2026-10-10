// Post-processing 2D: delikatne rybie oko (barrel distortion).
//
// Klatka idzie najpierw na teksturę pośrednią (`PostFx::scene`), a ten
// pass zamienia ją na obraz końcowy. To jedyne miejsce, w którym można
// zmienić położenie piksela — geometria jest już zapisana na teksturze.
//
// UWAGA: wzór musi zgadzać się z `fisheye_uv` w `postfx.rs`
// (testy CPU przenoszą wykrywanie rozjazdu na `cargo test`).

struct Params {
    // x = siła efektu (0 = brak), y = proporcje kadru (w/h), z, w = rezerwa.
    p: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var scene_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Pełnoekranowy trójkąt: trzy wierzchołki pokrywają cały kadr
// (tańszy niż kwadrat — brak rastrowania przekątnej).
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = corners[index];

    var out: VsOut;
    out.position = vec4<f32>(p, 0.0, 1.0);
    // NDC rośnie w górę, a UV w dół — oś Y trzeba odwrócić.
    out.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let strength = params.p.x;
    let aspect = params.p.y;

    // Środek kadru w jednostkach „połowy wysokości". Korekta proporcji
    // jest konieczna: bez niej okrągłe rybie oko rozciągałoby się
    // w elipsę przy oknie 16:9.
    var c = in.uv - vec2<f32>(0.5, 0.5);
    c.x = c.x * aspect;
    let r2 = dot(c, c);

    // Najdalszy róg kadru — mianownik normalizujący. Dzięki niemu
    // próbka nigdy nie wychodzi poza teksturę: brak smarowania krawędzi
    // i czarnych plam nawet przy dużej sile efektu.
    let corner = 0.25 * (aspect * aspect + 1.0);

    // Rybie oko: skala próbki ROŚNIE z promieniem (1 w narożniku,
    // mniej w środku), więc środek kadru wybrzusza się najbardziej,
    // a proste linie wyginają się jak deski beczki. Przy `strength = 0`
    // wzór redukuje się do tożsamości.
    let scale = (1.0 + strength * r2) / (1.0 + strength * corner);
    var d = c * scale;
    d.x = d.x / aspect;

    return textureSample(scene, scene_sampler, vec2<f32>(0.5) + d);
}
