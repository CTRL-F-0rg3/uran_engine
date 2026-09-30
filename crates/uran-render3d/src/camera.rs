//! Kamera perspektywiczna 3D.
//!
//! Trzyma pozycję i cel, a macierz `view_proj` liczy dopiero przy
//! rysowaniu — dzięki temu poruszanie kamerą nie kosztuje nic poza
//! jednym mnożeniem macierzy na klatkę.

use uran_math::{Mat4, Vec3};

/// Kamera 3D: pozycja, cel i pole widzenia.
///
/// W prawej ręce: patrzymy wzdłuż osi `-Z`, jak w OpenGL/wgpu.
#[derive(Debug, Clone, Copy)]
pub struct Camera3d {
    /// Pozycja oka w świecie.
    pub position: Vec3,
    /// Punkt, na który patrzymy (nie musi być znormalizowany).
    pub target: Vec3,
    /// Kąt pionowy pola widzenia w radianach.
    pub fov_y: f32,
    /// Stosunek szerokości do wysokości (aspect).
    pub aspect: f32,
    /// Odległość bliższej i dalszej płaszczyzny obcinania.
    pub near: f32,
    pub far: f32,
}

impl Default for Camera3d {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 3.0, 8.0),
            target: Vec3::ZERO,
            // 60° to wygodny kompromis: widać szeroko, ale nie ma
            // charakterystycznego „rybiego oka" z 90°
            fov_y: std::f32::consts::FRAC_PI_3,
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 1000.0,
        }
    }
}

impl Camera3d {
    /// Ustawia proporcje obrazu (po resize okna).
    pub fn set_aspect(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.0 {
            self.aspect = aspect;
        }
    }

    /// Kierunek patrzenia (jednostkowy).
    pub fn forward(&self) -> Vec3 {
        (self.target - self.position).normalize_or_zero()
    }

    /// Wektor „w górę" świata — w kamerze wolnajazdowej to po prostu +Y.
    pub fn up(&self) -> Vec3 {
        Vec3::Y
    }
    /// Wektor „w prawo" (poprzeczny do kierunku patrzenia).
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize_or_zero()
    }

    /// Macierz świata (kolumnowa) -> NDC.
    ///
    /// Kolejność `look_at * perspective` wynika z tego, że w
    /// kolumnowym zapisie `A * B * v` najpierw działa `B` — czyli
    /// najpierw przesuwamy punkt do przestrzeni oka, potem rzutujemy.
    pub fn view_proj(&self) -> Mat4 {
        let view = Mat4::look_at_rh(self.position, self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fov_y, self.aspect, self.near, self.far);
        proj * view
    }

    /// Sama macierz widoku (świat -> przestrzeń oka).
    ///
    /// Osobno od [`Self::view_proj`], bo G-Bufer trzyma normalne
    /// w przestrzeni oka, a shader nieba potrzebuje odwrotności całej
    /// macierzy rzutowania. Liczenie `look_at` drugi raz w shaderze
    /// nie wchodzi w grę — koszt jest po stronie GPU na każdy piksel.
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, Vec3::Y)
    }

    /// Tangens połowy kąta widzenia w pionie.
    ///
    /// Wpisujemy go do uniformu sceny, bo passy pełnoekranowe muszą
    /// odtworzyć promień przez piksel, a liczenie `tan` w każdym z nich
    /// powielałoby tę samą definicję pola widzenia w czterech miejscach.
    pub fn tan_half_fov(&self) -> f32 {
        (self.fov_y * 0.5).tan()
    }

    /// Promień z oka w kierunku `dir` (kierunek NIE musi być jednostkowy —
    /// normalizujemy tutaj, więc można podać różnicę `target - position`).
    pub fn ray(&self, dir: Vec3) -> Ray {
        Ray {
            origin: self.position,
            dir: dir.normalize_or_zero(),
        }
    }

    /// Promień przez punkt ekranu w znormalizowanych współrzędnych.
    ///
    /// `ndc` to standardowy zakres `[-1, 1]` (oś Y w górę, przeciwnie do
    /// konwencji ekranowej silnika). Środek to `(0,0)`, prawy górny róg
    /// to `(1,1)`.
    pub fn ray_through_ndc(&self, ndc_x: f32, ndc_y: f32) -> Ray {
        let f = self.forward();
        let r = self.right();
        let u = f.cross(r);
        // tangens połowy FOV-a: podstawa trójkąta na płaszczyźnie bliższej
        let ty = (self.fov_y * 0.5).tan();
        let tx = ty * self.aspect;
        let dir = (f + r * (ndc_x * tx) + u * (ndc_y * ty)).normalize_or_zero();
        self.ray(dir)
    }
}

/// Promień w świecie (początek + kierunek jednostkowy).
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    /// Punkt w odległości `t` od początku.
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Promień przesunięty o wektor `offset` (np. o pozycję lufy).
    pub fn shifted(&self, offset: Vec3) -> Ray {
        Ray {
            origin: self.origin + offset,
            dir: self.dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_camera_looks_from_above_towards_origin() {
        // kamera startuje NAD osią Z i patrzy w dół-ku środkowi, więc
        // kierunek ma ujemne Y i Z
        let c = Camera3d::default();
        let f = c.forward();
        assert!(f.z < -0.9, "kamera z +Z patrzy na -Z, a jest {f:?}");
        assert!(f.y < 0.0, "kamera jest nad osią, więc patrzy w dół: {f:?}");
    }

    #[test]
    fn right_is_perpendicular_to_forward() {
        let c = Camera3d::default();
        let dot = c.right().dot(c.forward());
        assert!(dot.abs() < 1e-5, "wektory nie są prostopadłe: {dot}");
    }

    #[test]
    fn aspect_guard_rejects_nonsense() {
        let mut c = Camera3d::default();
        let before = c.aspect;
        c.set_aspect(0.0);
        c.set_aspect(f32::NAN);
        assert_eq!(c.aspect, before, "złe aspect nie mogą psuć macierzy");
    }

    #[test]
    fn centre_ray_matches_forward() {
        let c = Camera3d::default();
        let r = c.ray_through_ndc(0.0, 0.0);
        assert!(
            r.dir.dot(c.forward()) > 0.999,
            "środek ekranu to kierunek patrzenia"
        );
    }

    #[test]
    fn rays_fan_out_towards_screen_edges() {
        let c = Camera3d::default();
        let centre = c.ray_through_ndc(0.0, 0.0);
        let right = c.ray_through_ndc(1.0, 0.0);
        let up = c.ray_through_ndc(0.0, 1.0);
        // krawędź ekranu musi zbaczać w bok, inaczej mamy równoległy rzut
        assert!(right.dir.dot(c.right()) > 0.2);
        // górna krawędź patrzy WYŻEJ niż środek, więc jej kierunek
        // ma mniejsze Y (kamera patrzy w dół) — stąd `-`
        assert!(up.dir.y < centre.dir.y, "górna krawędź nie idzie w górę");
    }

    #[test]
    fn ray_direction_is_unit_length() {
        // `Ray` NIE normalizuje w konstruktorze — to świadome: wołujący
        // przekazuje już znormalizowany kierunek z kamery.
        let r = Ray {
            origin: Vec3::ZERO,
            dir: Vec3::new(0.0, 0.0, 7.0).normalize(),
        };
        let n = r.at(2.0);
        assert!(
            (n.length() - 2.0).abs() < 1e-5,
            "punkt {} nie leży 2 od początku",
            n.length()
        );
    }
}
