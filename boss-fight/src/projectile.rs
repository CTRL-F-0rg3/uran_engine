//! Pociski: zarówno atak gracza, jak i pociski bossa.
//!
//! Jeden typ struktury obsługuje obie strony walki — różni je
//! `owner`. To świadomy wybór: gdyby były dwie osobne listy, kod
//! rysowania, kolizji i sprzątania musiałby się powtarzać, a
//! najbardziej prawdopodobny błąd („pocisk gracza zadał obrażenia
//! graczowi") byłby niemal niemożliwy do zauważenia.
//!
//! Wszystko w jednostkach świata i sekundach.

use uran_math::{Rect, Vec2};

/// Kto wystrzelił pocisk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Player,
    Boss,
}

/// Pocisk: prostokąt, prędkość, obrażenia i czas życia.
#[derive(Debug, Clone, Copy)]
pub struct Projectile {
    /// Środek pocisku.
    pub pos: Vec2,
    /// Prędkość w jednostkach na sekundę.
    pub vel: Vec2,
    /// Kto wystrzelił.
    pub owner: Owner,
    /// Ile obrażeń zadaje.
    pub damage: f32,
    /// Prostokąt pocisku.
    pub size: Vec2,
    /// Ile sekund zostało do zniknięcia.
    pub life: f32,
    /// Czy pocisk jest ciężki (przebija, nie znika po trafieniu).
    pub piercing: bool,
}

impl Projectile {
    /// Prostokąt kolizji pocisku.
    pub fn rect(&self) -> Rect {
        Rect::from_center(self.pos, self.size)
    }

    /// Krok: ruch po linii prostej i starzenie.
    ///
    /// Pociski nie reagują na grawitację ani kafle — celowo. Boss
    /// strzela prostymi, a gracz może strzelać pod górę; gdyby pocisk
    /// spadał, jego tor przestałby być przewidywalny, a unikanie
    /// ataków stałoby się nieuczciwe.
    pub fn step(&mut self, dt: f32) {
        self.pos += self.vel * dt;
        self.life -= dt;
    }

    /// Czy pocisk wygasł.
    pub fn is_dead(&self) -> bool {
        self.life <= 0.0
    }
}

/// Ile sekund żyje pocisk — bez tego pociski bossa leciałyby w
/// nieskończoność i kumulowały się w pamięci.
pub const BULLET_LIFE: f32 = 6.0;

/// Czy pocisk gracza trafia bossa.
pub fn player_hits_boss(bullet: &Projectile, boss_rect: Rect) -> bool {
    bullet.owner == Owner::Player && bullet.rect().intersects(boss_rect)
}

/// Czy pocisk bossa trafia gracza.
pub fn boss_hits_player(bullet: &Projectile, player_rect: Rect) -> bool {
    bullet.owner == Owner::Boss && bullet.rect().intersects(player_rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bullet(owner: Owner) -> Projectile {
        Projectile {
            pos: Vec2::ZERO,
            vel: Vec2::new(200.0, 0.0),
            owner,
            damage: 10.0,
            size: Vec2::splat(8.0),
            life: BULLET_LIFE,
            piercing: false,
        }
    }

    #[test]
    fn moves_along_its_velocity() {
        let mut b = bullet(Owner::Player);
        b.step(0.5);
        assert!((b.pos.x - 100.0).abs() < 1e-3, "x = {}", b.pos.x);
        assert_eq!(b.pos.y, 0.0);
    }

    #[test]
    fn dies_after_its_lifetime() {
        let mut b = bullet(Owner::Player);
        assert!(!b.is_dead());
        for _ in 0..(BULLET_LIFE * 100.0) as u32 {
            b.step(0.01);
        }
        assert!(b.is_dead(), "pocisk nie zniknął");
    }

    #[test]
    fn owner_prevents_friendly_fire() {
        let p = bullet(Owner::Player);
        let boss = Rect::from_xywh(-10.0, -10.0, 20.0, 20.0);
        // Pocisk gracza trafia bossa…
        assert!(player_hits_boss(&p, boss));
        // …ale nie gracza, nawet gdy stoi w tym samym miejscu.
        assert!(!boss_hits_player(&p, boss));
    }

    #[test]
    fn boss_bullet_only_hits_the_player() {
        let b = bullet(Owner::Boss);
        let r = Rect::from_xywh(-10.0, -10.0, 20.0, 20.0);
        assert!(boss_hits_player(&b, r));
        assert!(!player_hits_boss(&b, r));
    }

    #[test]
    fn miss_when_far_away() {
        let b = bullet(Owner::Player);
        assert!(!player_hits_boss(&b, Rect::from_xywh(500.0, 0.0, 10.0, 10.0)));
    }
}
