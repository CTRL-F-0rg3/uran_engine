use glam::Vec2;

/// Prostokąt osiowy w przestrzeni 2D (dolny róg = `min`, górny = `max`).
///
/// Domyślnie zakładamy oś Y skierowaną w górę — tak jak w całym silniku.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub const ZERO: Self = Self::new(Vec2::ZERO, Vec2::ZERO);

    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    /// `x, y` to lewy dolny róg.
    pub const fn from_xywh(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::new(Vec2::new(x, y), Vec2::new(x + w, y + h))
    }

    pub fn from_center(center: Vec2, size: Vec2) -> Self {
        let half = size * 0.5;
        Self::new(center - half, center + half)
    }

    /// Środek w (0, 0) — wygodne do rysowania sprite'ów „na pozycji".
    pub fn from_size(size: Vec2) -> Self {
        let half = size * 0.5;
        Self::new(-half, half)
    }

    /// Odtwarza `Rect` z wymiarów obrazu (anchor: lewy dolny róg).
    pub fn from_size_at(origin: Vec2, size: Vec2) -> Self {
        Self::new(origin, Vec2::new(origin.x + size.x, origin.y + size.y))
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    pub fn left(&self) -> f32 {
        self.min.x
    }

    pub fn bottom(&self) -> f32 {
        self.min.y
    }

    pub fn right(&self) -> f32 {
        self.max.x
    }

    pub fn top(&self) -> f32 {
        self.max.y
    }

    pub fn is_empty(&self) -> bool {
        self.max.x <= self.min.x || self.max.y <= self.min.y
    }

    /// Rogi w kolejności: lewy-dolny, prawy-dolny, prawy-górny, lewy-górny.
    pub fn corners(&self) -> [Vec2; 4] {
        [
            Vec2::new(self.min.x, self.min.y),
            Vec2::new(self.max.x, self.min.y),
            Vec2::new(self.max.x, self.max.y),
            Vec2::new(self.min.x, self.max.y),
        ]
    }

    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.min.x && point.x <= self.max.x && point.y >= self.min.y && point.y <= self.max.y
    }

    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.min.x >= self.min.x
            && other.max.x <= self.max.x
            && other.min.y >= self.min.y
            && other.max.y <= self.max.y
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        if min.x < max.x && min.y < max.y {
            Some(Rect::new(min, max))
        } else {
            None
        }
    }

    /// Najmniejszy prostokąt zawierający oba.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect::new(self.min.min(other.min), self.max.max(other.max))
    }

    /// Powiększa prostokąt o podany punkt (jak `cv::boundingRect`).
    pub fn expand(&self, point: Vec2) -> Rect {
        Rect::new(self.min.min(point), self.max.max(point))
    }

    pub fn expand_by(&self, dx: f32, dy: f32) -> Rect {
        let d = Vec2::new(dx, dy);
        Rect::new(self.min - d, self.max + d)
    }

    pub fn translate(&self, offset: Vec2) -> Rect {
        Rect::new(self.min + offset, self.max + offset)
    }

    /// Rozmiar nakładania się dwóch AABB (0 jeśli się nie przecinają).
    /// Przydatne do prostej fizyki 2D (AABB resolution).
    pub fn overlap(&self, other: &Rect) -> Vec2 {
        let overlap_x = (self.max.x.min(other.max.x) - self.min.x.max(other.min.x)).max(0.0);
        let overlap_y = (self.max.y.min(other.max.y) - self.min.y.max(other.min.y)).max(0.0);
        Vec2::new(overlap_x, overlap_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let r = Rect::from_xywh(10.0, 20.0, 30.0, 40.0);
        assert_eq!(r.size(), Vec2::new(30.0, 40.0));
        assert_eq!(r.center(), Vec2::new(25.0, 40.0));
        assert_eq!(r.left(), 10.0);
        assert_eq!(r.top(), 60.0);
        assert!(!r.is_empty());
    }

    #[test]
    fn from_center_is_symmetric() {
        let r = Rect::from_center(Vec2::new(5.0, 5.0), Vec2::new(4.0, 2.0));
        assert_eq!(r.min, Vec2::new(3.0, 4.0));
        assert_eq!(r.max, Vec2::new(7.0, 6.0));
        assert_eq!(Rect::from_size(Vec2::splat(2.0)).min, Vec2::splat(-1.0));
    }

    #[test]
    fn containment() {
        let r = Rect::from_xywh(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Vec2::new(5.0, 5.0)));
        assert!(r.contains(Vec2::new(0.0, 0.0)));
        assert!(!r.contains(Vec2::new(-0.1, 5.0)));
        assert!(r.contains_rect(&Rect::from_xywh(1.0, 1.0, 2.0, 2.0)));
        assert!(!r.contains_rect(&Rect::from_xywh(9.0, 9.0, 2.0, 2.0)));
    }

    #[test]
    fn intersections() {
        let a = Rect::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = Rect::from_xywh(5.0, 5.0, 10.0, 10.0);
        let c = Rect::from_xywh(20.0, 20.0, 1.0, 1.0);
        assert!(a.intersects(&b));
        assert!(!a.intersects(&c));
        assert_eq!(
            a.intersection(&b),
            Some(Rect::from_xywh(5.0, 5.0, 5.0, 5.0))
        );
        assert_eq!(a.intersection(&c), None);
        assert_eq!(a.union(&c), Rect::from_xywh(0.0, 0.0, 21.0, 21.0));
    }

    #[test]
    fn overlap_and_expand() {
        let a = Rect::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = Rect::from_xywh(8.0, 3.0, 10.0, 10.0);
        assert_eq!(a.overlap(&b), Vec2::new(2.0, 7.0));
        assert_eq!(a.expand(Vec2::new(-1.0, 20.0)).min, Vec2::new(-1.0, 0.0));
        assert_eq!(a.expand_by(1.0, 2.0).min, Vec2::new(-1.0, -2.0));
        assert_eq!(a.translate(Vec2::splat(5.0)).min, Vec2::splat(5.0));
    }
}