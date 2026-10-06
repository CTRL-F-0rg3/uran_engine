//! Generatory kształtów 2D: wszystko, co da się narysować wektorowo
//! (wielokąty, okręgi, linie, gwiazdy) oraz triangulacja metodą „uszy".

use uran_ecs::{Mesh, Vertex};
use uran_math::{Color, Rect, Vec2};

/// Podpisane pole wielokąta: > 0 dla kolejności przeciwnej do zegara (CCW).
fn signed_area(points: &[Vec2]) -> f32 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Czy punkt leży wewnątrz trójkąta (zakłada CCW).
fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = cross(b - a, p - a);
    let d2 = cross(c - b, p - b);
    let d3 = cross(a - c, p - c);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

/// Trianguluje prosty (bez przecięć) wielokąt metodą uszy (ear clipping).
///
/// Zwraca indeksy trójkątów odnoszące się do podanych punktów. Dla wielokątów
/// wklęsłych (np. gwiazda) działa poprawnie, w przeciwieństwie do
/// triangulacji „wachlarzem". Dla kształtów samoprzecinających się
/// zwraca pustą listę (lepiej nie rysować niż rysować śmieci).
///
/// **Powtarzające się sąsiednie punkty są usuwane.** To nie kosmetyka:
/// kształt „pigułka" (`rounded_rect` z `radius == połowa wysokości`) ma
/// dwa środki łuków dokładnie na sobie, więc bez tego kroku ear-clipping
/// nie znajduje żadnego ucha i zwraca pustą listę — czyli pasek HUD-u
/// po prostu znikałby z ekranu zamiast być zaokrąglony.
pub fn triangulate(points: &[Vec2]) -> Vec<u32> {
    let mut indices: Vec<u32> = Vec::new();
    if points.len() < 3 {
        return indices;
    }
    // Zdegenerowany wielokąt (wszystkie punkty na jednej prostej albo
    // identyczne) nie ma co triangulować — inaczej powstałby "trójkąt"
    // o zerowym polu.
    if signed_area(&points).abs() < 1e-9 {
        return indices;
    }

    // Usuwamy duplikaty sąsiadów, trzymając mapowanie na indeksy wejścia,
    // bo zwracane indeksy muszą wskazywać **podane** punkty.
    let mut poly: Vec<Vec2> = Vec::with_capacity(points.len());
    let mut source: Vec<usize> = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        if poly
            .last()
            .is_some_and(|last| (*last - *p).length_squared() < 1e-12)
        {
            continue;
        }
        poly.push(*p);
        source.push(i);
    }
    // Punkt powtarzający się na styku końca i początku listy.
    while poly.len() > 1 && (poly[0] - poly[poly.len() - 1]).length_squared() < 1e-12 {
        poly.pop();
        source.pop();
    }
    if poly.len() < 3 {
        return indices;
    }

    // algorytm zakłada kolejność CCW
    if signed_area(&poly) < 0.0 {
        poly.reverse();
        source.reverse();
    }

    let mut remaining: Vec<usize> = (0..poly.len()).collect();
    // limit iteracji chroni przed zapętleniem na geometrii degeneracyjnej
    let mut guard = poly.len() * poly.len() + 16;

    while remaining.len() > 3 && guard > 0 {
        guard -= 1;
        let mut clipped = false;

        for i in 0..remaining.len() {
            let prev = remaining[(i + remaining.len() - 1) % remaining.len()];
            let curr = remaining[i];
            let next = remaining[(i + 1) % remaining.len()];

            let (a, b, c) = (poly[prev], poly[curr], poly[next]);
            // tylko wypukłe „uszy"
            if cross(b - a, c - b) <= 0.0 {
                continue;
            }
            // żaden pozostały wierzchołek nie może leżeć w tym trójkącie
            let contains = remaining.iter().any(|&other| {
                other != prev
                    && other != curr
                    && other != next
                    && point_in_triangle(poly[other], a, b, c)
            });
            if contains {
                continue;
            }

            indices.extend_from_slice(&[
                source[prev] as u32,
                source[curr] as u32,
                source[next] as u32,
            ]);
            remaining.remove(i);
            clipped = true;
            break;
        }

        if !clipped {
            return Vec::new();
        }
    }

    if remaining.len() == 3 {
        indices.extend_from_slice(&[
            source[remaining[0]] as u32,
            source[remaining[1]] as u32,
            source[remaining[2]] as u32,
        ]);
    }
    indices
}

/// Wypełniony wielokąt o jednolitym kolorze.
pub fn polygon(points: &[Vec2], color: Color) -> Mesh {
    let indices = triangulate(points);
    Mesh::new(
        points.iter().map(|p| Vertex::new(*p, color)).collect(),
        indices,
    )
}

/// Okrąg wypełniony, złożony z `segments` trójkątów.
pub fn circle(center: Vec2, radius: f32, segments: u32, color: Color) -> Mesh {
    let segments = segments.max(3) as usize;
    let mut vertices = Vec::with_capacity(segments + 1);
    vertices.push(Vertex::new(center, color));
    for i in 0..segments {
        let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
        vertices.push(Vertex::new(
            center + Vec2::new(angle.cos(), angle.sin()) * radius,
            color,
        ));
    }
    let indices = (0..segments as u32)
        .flat_map(|i| [0, i + 1, ((i + 1) % segments as u32) + 1])
        .collect();
    Mesh::new(vertices, indices)
}

/// Pierścień (okrąg z dziurą) — grubość liczona od środka do krawędzi wewnętrznej.
pub fn ring(center: Vec2, radius: f32, thickness: f32, segments: u32, color: Color) -> Mesh {
    let segments = segments.max(3);
    let inner = (radius - thickness).max(0.0);
    let mut vertices = Vec::with_capacity(segments as usize * 2);
    for i in 0..segments {
        let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
        let dir = Vec2::new(angle.cos(), angle.sin());
        vertices.push(Vertex::new(center + dir * radius, color));
        vertices.push(Vertex::new(center + dir * inner, color));
    }
    let mut indices = Vec::with_capacity(segments as usize * 6);
    for i in 0..segments {
        let a = i * 2;
        let b = ((i + 1) % segments) * 2;
        indices.extend_from_slice(&[a, b, a + 1, b, b + 1, a + 1]);
    }
    Mesh::new(vertices, indices)
}

/// Prostokąt z zaokrąglonymi rogami (promień w jednostkach świata).
pub fn rounded_rect(rect: Rect, radius: f32, segments_per_corner: u32, color: Color) -> Mesh {
    let radius = radius.clamp(0.0, rect.width().min(rect.height()) * 0.5);
    if radius <= f32::EPSILON {
        return Mesh::quad(rect.corners(), color);
    }
    let segments = segments_per_corner.max(1) as usize;
    // środki łuków: prawy-dolny, prawy-górny, lewy-górny, lewy-dolny
    let centers = [
        Vec2::new(rect.max.x - radius, rect.min.y + radius),
        Vec2::new(rect.max.x - radius, rect.max.y - radius),
        Vec2::new(rect.min.x + radius, rect.max.y - radius),
        Vec2::new(rect.min.x + radius, rect.min.y + radius),
    ];
    // kąty startowe w stopniach (0 = prawo, 90 = góra) — oś Y w górę
    let starts = [-90.0f32, 0.0, 90.0, 180.0];

    let mut points = Vec::with_capacity(segments * 4);
    for (center, start) in centers.iter().zip(starts) {
        for i in 0..=segments {
            let angle = (start + 90.0 * (i as f32 / segments as f32)).to_radians();
            points.push(*center + Vec2::new(angle.cos(), angle.sin()) * radius);
        }
    }
    polygon(&points, color)
}

/// Odcinek o zadanej grubości (jako czworokąt).
pub fn thick_line(from: Vec2, to: Vec2, width: f32, color: Color) -> Mesh {
    let half = width * 0.5;
    let direction = to - from;
    let len = direction.length();
    if len < f32::EPSILON {
        return Mesh::quad(
            [
                from + Vec2::new(-half, -half),
                from + Vec2::new(half, -half),
                from + Vec2::new(half, half),
                from + Vec2::new(-half, half),
            ],
            color,
        );
    }
    let normal = Vec2::new(-direction.y, direction.x).normalize() * half;
    Mesh::quad(
        [from + normal, to + normal, to - normal, from - normal],
        color,
    )
}

/// Łamana o zadanej grubości (każdy odcinek osobnym czworokątem).
pub fn polyline(points: &[Vec2], width: f32, color: Color) -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for pair in points.windows(2) {
        let segment = thick_line(pair[0], pair[1], width, color);
        let base = vertices.len() as u32;
        vertices.extend(segment.vertices);
        indices.extend(segment.indices.into_iter().map(|i| i + base));
    }
    Mesh::new(vertices, indices)
}

/// Kontur prostokąta o zadanej grubości.
pub fn rect_outline(rect: Rect, width: f32, color: Color) -> Mesh {
    let [bl, br, tr, tl] = rect.corners();
    polyline(&[bl, br, tr, tl, bl], width, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Suma pól trójkątów (ze znakiem) — do sprawdzania triangulacji.
    fn triangle_area_sum(m: &Mesh) -> f32 {
        let mut total = 0.0;
        for tri in m.indices.chunks(3) {
            let a = Vec2::from_array(m.vertices[tri[0] as usize].position);
            let b = Vec2::from_array(m.vertices[tri[1] as usize].position);
            let c = Vec2::from_array(m.vertices[tri[2] as usize].position);
            total += ((b - a).perp_dot(c - a)) * 0.5;
        }
        total.abs()
    }

    #[test]
    fn triangulate_square() {
        let square = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let indices = triangulate(&square);
        assert_eq!(indices.len(), 6, "kwadrat = 2 trójkąty");
        assert!(indices.iter().all(|i| (*i as usize) < square.len()));
    }

    #[test]
    fn triangulate_works_for_concave() {
        // kształt „L" — wklęsły, metoda wachlarzowa by tu zawiodła
        let l_shape = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 4.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(4.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let indices = triangulate(&l_shape);
        assert_eq!(indices.len(), 12, "6 wierzchołków = 4 trójkąty");
        let mesh = polygon(&l_shape, Color::WHITE);
        // pole L-kształtu = 10*4 + 4*6 = 64
        assert!((triangle_area_sum(&mesh) - 64.0).abs() < 0.01);
    }

    #[test]
    fn triangulate_handles_both_windings() {
        let mut points = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let ccw = triangulate(&points).len();
        points.reverse();
        assert_eq!(triangulate(&points).len(), ccw);
    }

    #[test]
    fn triangulate_rejects_degenerate() {
        assert!(triangulate(&[Vec2::ZERO, Vec2::X]).is_empty());
        // wszystkie punkty na jednej prostej
        let collinear = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 0.0),
        ];
        assert!(triangulate(&collinear).is_empty());
    }

    #[test]
    fn circle_has_expected_area() {
        let m = circle(Vec2::ZERO, 10.0, 64, Color::WHITE);
        assert_eq!(m.triangle_count(), 64);
        let area = triangle_area_sum(&m);
        // pole koła to pi*r^2; wielokąt 64-kąt traci ok. 0.13%
        let expected = std::f32::consts::PI * 100.0;
        assert!(
            (area - expected).abs() < expected * 0.002,
            "pole = {area}, oczekiwano ~{expected}"
        );
        assert!(m.validate());
    }

    #[test]
    fn ring_is_annulus() {
        let m = ring(Vec2::ZERO, 10.0, 2.0, 32, Color::WHITE);
        let area = triangle_area_sum(&m);
        let expected = std::f32::consts::PI * (100.0 - 64.0);
        assert!(
            (area - expected).abs() < 1.0,
            "pole = {area}, oczekiwano {expected}"
        );
    }

    #[test]
    fn rounded_rect_without_radius_is_quad() {
        let m = rounded_rect(Rect::from_xywh(0.0, 0.0, 10.0, 10.0), 0.0, 4, Color::WHITE);
        assert_eq!(m.triangle_count(), 2);
    }

    #[test]
    fn rounded_rect_area_is_smaller_than_plain() {
        let rect = Rect::from_xywh(0.0, 0.0, 20.0, 20.0);
        let rounded = rounded_rect(rect, 6.0, 8, Color::WHITE);
        assert!(rounded.validate());
        let area = triangle_area_sum(&rounded);
        assert!(
            area < 400.0 && area > 340.0,
            "zaokrąglenia zjadły narożniki: {area}"
        );
    }

    /// Prostokąt z `radius == połowa wysokości` to „pigułka": dwa środki
    /// łuków lądują dokładnie na sobie. Bez usuwania duplikatów
    /// ear-clipping nie znajduje ucha i zwraca pustą siatkę — pasek HUD-u
    /// znikałby z ekranu. To dokładnie ten kształt, którego używają paski
    /// postępu i statystyk.
    #[test]
    fn pill_shaped_rect_is_not_dropped() {
        let rect = Rect::from_xywh(0.0, 0.0, 60.0, 8.0);
        let m = rounded_rect(rect, 4.0, 8, Color::WHITE);
        assert!(m.triangle_count() > 0, "pigułka musi się triangulować");
        assert!(m.validate());

        // pole kapsuły = prostokąt (w - 2r) * h + koło o promieniu r;
        // inaczej: w*h - (4 - pi) * r^2
        let area = triangle_area_sum(&m);
        let r = 4.0_f32;
        let expected = 60.0 * 8.0 - (4.0 - std::f32::consts::PI) * r * r;
        assert!(
            (area - expected).abs() < 1.0,
            "pole = {area}, oczekiwano ~{expected}"
        );
    }

    /// Duplikaty **sąsiadujących** punktów nie mogą psuć siatki — tak
    /// dokładnie generuje je `rounded_rect`, gdy dwa sąsiednie rogi mają
    /// ten sam środek łuku.
    #[test]
    fn duplicate_points_are_deduplicated() {
        let square = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 0.0), // duplikat sąsiada
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
            Vec2::new(0.0, 10.0), // duplikat sąsiada
            Vec2::new(0.0, 0.0),  // duplikat pierwszego (zawiązanie)
        ];
        let indices = triangulate(&square);
        assert!(!indices.is_empty(), "duplikaty nie mogą wyzerować siatki");
        assert!(
            indices.iter().all(|i| (*i as usize) < square.len()),
            "indeksy muszą wskazywać wejściowe punkty"
        );
        let area = triangle_area_sum(&polygon(&square, Color::WHITE));
        assert!((area - 100.0).abs() < 0.01, "pole = {area}");
    }

    /// Wielokąt podany w kolejności zgodnej z zegarem (CW) też musi dać
    /// poprawne pole — indeksy wskazują wejście, nie odwróconą tablicę.
    #[test]
    fn clockwise_polygon_keeps_correct_indices() {
        let mut points = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let ccw_area = triangle_area_sum(&polygon(&points, Color::WHITE));
        points.reverse();
        let cw_area = triangle_area_sum(&polygon(&points, Color::WHITE));
        assert!((ccw_area - 100.0).abs() < 0.01, "pole CCW = {ccw_area}");
        assert!((cw_area - 100.0).abs() < 0.01, "pole CW = {cw_area}");
    }

    #[test]
    fn thick_line_covers_length_and_width() {
        let m = thick_line(Vec2::ZERO, Vec2::new(10.0, 0.0), 4.0, Color::WHITE);
        assert!((triangle_area_sum(&m) - 40.0).abs() < 0.01);
    }

    #[test]
    fn polyline_sums_segments() {
        let m = polyline(
            &[Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)],
            2.0,
            Color::WHITE,
        );
        assert_eq!(m.triangle_count(), 4, "2 odcinki po 2 trójkąty");
        assert!((triangle_area_sum(&m) - 40.0).abs() < 0.01);
    }

    #[test]
    fn rect_outline_is_closed() {
        let m = rect_outline(Rect::from_xywh(0.0, 0.0, 10.0, 10.0), 1.0, Color::WHITE);
        assert_eq!(m.triangle_count(), 8);
        // obwód 40 * grubość 1 = 40
        assert!((triangle_area_sum(&m) - 40.0).abs() < 0.01);
    }

    #[test]
    fn star_is_concave_but_triangulates() {
        let m = star(Vec2::ZERO, 5, 0.4, 10.0, Color::WHITE);
        assert!(m.validate());
        assert_eq!(
            m.triangle_count(),
            8,
            "5 ramion = 10 wierzchołków = 8 trójkątów"
        );
        assert!(triangle_area_sum(&m) > 0.0);
    }

    #[test]
    fn empty_polygon_produces_nothing() {
        let m = polygon(&[Vec2::ZERO], Color::WHITE);
        assert!(m.is_empty());
    }
}

/// Gwiazda o `points` ramionach. `inner_ratio` = promień wewnętrzny / zewnętrzny.
pub fn star(center: Vec2, points: u32, inner_ratio: f32, radius: f32, color: Color) -> Mesh {
    let points = points.max(3);
    let mut pts = Vec::with_capacity(points as usize * 2);
    for i in 0..points * 2 {
        let r = if i % 2 == 0 {
            radius
        } else {
            radius * inner_ratio
        };
        let angle = (i as f32 / (points * 2) as f32) * std::f32::consts::TAU;
        pts.push(center + Vec2::new(angle.cos(), angle.sin()) * r);
    }
    polygon(&pts, color)
}
