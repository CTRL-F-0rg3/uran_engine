//! Fizyka 2D — wygodne opakowanie [rapier2d].
//!
//! `uran-physics` ma dwa poziomy: prosty AABB ([`crate::CollisionWorld`]) do
//! statycznych przeszkód i ten moduł — pełną fizykę rapiera: ciała dynamiczne,
//! grawitację, pęd, impulsy i kontakty.
//!
//! ```ignore
//! use uran_engine::prelude::*;
//!
//! let mut phys = PhysicsWorld::new(Vec2::new(0.0, -980.0)); // grawitacja
//! let podloga = phys.add_static_box(Vec2::new(0.0, 0.0), Vec2::new(100.0, 10.0));
//! let kula    = phys.add_dynamic_box(Vec2::new(0.0, 50.0), Vec2::new(8.0, 8.0));
//!
//! // co klatkę:
//! phys.apply_impulse(&kula, Vec2::new(200.0, 0.0), true);
//! phys.step(ctx.dt());
//! let pos = phys.position(&kula); // pozycja środka w jednostkach świata
//! ```

use rapier2d::prelude::*;
use uran_math::Vec2;

/// Rodzaj ciała — jak reaguje na siły i grawitację.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// Porusza się pod wpływem grawitacji, impulsów i sił.
    Dynamic,
    /// Nieruchoma przeszkoda (ściana, podłoga).
    Fixed,
}

/// Ciało fizyczne: para „ciało sztywne + collider" w rapierze.
///
/// Trzymasz je (jest tanie — to dwa uchwyty), żeby potem odczytywać/zmieniać
/// stan: [`PhysicsWorld::position`], [`PhysicsWorld::set_velocity`], itd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsBody {
    /// Uchwyt ciała sztywnego (dynamika, prędkość, masa).
    pub body: RigidBodyHandle,
    /// Uchwyt collidera (kształt do kolizji).
    pub collider: ColliderHandle,
}

/// Świat fizyki 2D — opakowanie rapiera z prostym, jednolitym API.
pub struct PhysicsWorld {
    gravity: rapier2d::math::Vector<f32>,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    integration: IntegrationParameters,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    ccd: CCDSolver,
    impulses: ImpulseJointSet,
    multibody: MultibodyJointSet,
}

impl PhysicsWorld {
    /// Nowy świat z zadaną grawitacją (np. `Vec2::new(0.0, -980.0)`).
    pub fn new(gravity: Vec2) -> Self {
        Self {
            gravity: to_vec(gravity),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            integration: IntegrationParameters::default(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            ccd: CCDSolver::new(),
            impulses: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
        }
    }

    /// Aktualna grawitacja (jednostki świata / s²).
    pub fn gravity(&self) -> Vec2 {
        from_vec(self.gravity)
    }

    /// Ustawia grawitację.
    pub fn set_gravity(&mut self, gravity: Vec2) {
        self.gravity = to_vec(gravity);
    }

    /// Dodaje ciało o danym środku i rozmiarze (prostopadłościan/kwadrat).
    ///
    /// `size` to pełny rozmiar w jednostkach świata (szerokość, wysokość).
    pub fn add(&mut self, center: Vec2, size: Vec2, kind: BodyKind) -> PhysicsBody {
        match kind {
            BodyKind::Dynamic => self.add_dynamic_box(center, size),
            BodyKind::Fixed => self.add_static_box(center, size),
        }
    }

    /// Dodaje **dynamiczne** ciało (kwadrat) — spada, odbija się, reaguje na siły.
    pub fn add_dynamic_box(&mut self, center: Vec2, size: Vec2) -> PhysicsBody {
        let body = RigidBodyBuilder::dynamic()
            .translation(to_vec(center))
            .build();
        let body_handle = self.bodies.insert(body);
        let collider = ColliderBuilder::cuboid(size.x * 0.5, size.y * 0.5).build();
        let collider_handle =
            self.colliders
                .insert_with_parent(collider, body_handle, &mut self.bodies);
        PhysicsBody {
            body: body_handle,
            collider: collider_handle,
        }
    }

    /// Dodaje **statyczną** przeszkodę (kwadrat) — ściana, podłoga.
    pub fn add_static_box(&mut self, center: Vec2, size: Vec2) -> PhysicsBody {
        let body = RigidBodyBuilder::fixed().translation(to_vec(center)).build();
        let body_handle = self.bodies.insert(body);
        let collider = ColliderBuilder::cuboid(size.x * 0.5, size.y * 0.5).build();
        let collider_handle =
            self.colliders
                .insert_with_parent(collider, body_handle, &mut self.bodies);
        PhysicsBody {
            body: body_handle,
            collider: collider_handle,
        }
    }

    /// Krok symulacji o `dt` sekund (zwykle `ctx.dt()`).
    pub fn step(&mut self, dt: f32) {
        self.integration.dt = dt.max(1e-4);
        self.pipeline.step(
            &self.gravity,
            &self.integration,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulses,
            &mut self.multibody,
            &mut self.ccd,
            None,
            &(),
            &(),
        );
    }

    /// Pozycja środka ciała w jednostkach świata.
    pub fn position(&self, body: &PhysicsBody) -> Vec2 {
        self.bodies
            .get(body.body)
            .map(|rb| from_vec(*rb.translation()))
            .unwrap_or(Vec2::ZERO)
    }

    /// Ustawia pozycję ciała (teleport). `wake` budzi ciało do dalszej symulacji.
    pub fn set_position(&mut self, body: &PhysicsBody, pos: Vec2, wake: bool) {
        if let Some(rb) = self.bodies.get_mut(body.body) {
            rb.set_translation(to_vec(pos), wake);
        }
    }

    /// Prędkość liniowa ciała (jednostki / s).
    pub fn velocity(&self, body: &PhysicsBody) -> Vec2 {
        self.bodies
            .get(body.body)
            .map(|rb| from_vec(*rb.linvel()))
            .unwrap_or(Vec2::ZERO)
    }

    /// Ustawia prędkość liniową.
    pub fn set_velocity(&mut self, body: &PhysicsBody, vel: Vec2, wake: bool) {
        if let Some(rb) = self.bodies.get_mut(body.body) {
            rb.set_linvel(to_vec(vel), wake);
        }
    }

    /// Pcha ciało natychmiast (skok, wystrzał). Impuls = zmiana pędu.
    pub fn apply_impulse(&mut self, body: &PhysicsBody, impulse: Vec2, wake: bool) {
        if let Some(rb) = self.bodies.get_mut(body.body) {
            rb.apply_impulse(to_vec(impulse), wake);
        }
    }

    /// Stała siła (np. wiatr) — działa przez cały czas, dopóki jej nie zrównoważy.
    pub fn apply_force(&mut self, body: &PhysicsBody, force: Vec2, wake: bool) {
        if let Some(rb) = self.bodies.get_mut(body.body) {
            rb.add_force(to_vec(force), wake);
        }
    }

    /// Zeruje siły i pęd obrotowy (użyteczne, gdy ciało ma się uspokoić).
    pub fn reset_forces(&mut self, body: &PhysicsBody, wake: bool) {
        if let Some(rb) = self.bodies.get_mut(body.body) {
            rb.reset_forces(wake);
        }
    }

    /// Usuwa ciało i jego collider ze świata.
    pub fn remove(&mut self, body: PhysicsBody) {
        self.colliders
            .remove(body.collider, &mut self.islands, &mut self.bodies, false);
        self.bodies.remove(
            body.body,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulses,
            &mut self.multibody,
            false,
        );
    }

    /// Liczba ciał w świecie.
    pub fn body_count(&self) -> usize {
        self.bodies.len()
    }
}

fn to_vec(v: Vec2) -> rapier2d::math::Vector<f32> {
    rapier2d::math::Vector::new(v.x, v.y)
}

fn from_vec(v: rapier2d::math::Vector<f32>) -> Vec2 {
    Vec2::new(v.x, v.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ciało dynamiczne pod grawitacją spada w dół.
    #[test]
    fn gravity_pulls_a_dynamic_body_down() {
        let mut w = PhysicsWorld::new(Vec2::new(0.0, -100.0));
        let b = w.add_dynamic_box(Vec2::ZERO, Vec2::new(2.0, 2.0));
        w.step(1.0 / 60.0);
        assert!(w.position(&b).y < 0.0, "ciało nie spadło: {:?}", w.position(&b));
    }

    /// Statyczna podłoga zatrzymuje spadające ciało.
    #[test]
    fn floor_stops_a_falling_body() {
        let mut w = PhysicsWorld::new(Vec2::new(0.0, -1000.0));
        w.add_static_box(Vec2::new(0.0, -5.0), Vec2::new(100.0, 1.0));
        let b = w.add_dynamic_box(Vec2::new(0.0, 5.0), Vec2::new(2.0, 2.0));

        for _ in 0..600 {
            w.step(1.0 / 60.0);
        }
        let pos = w.position(&b);
        assert!(pos.y > -4.5, "ciało wpadło pod podłogę: {pos:?}");
        assert!(pos.y < 5.0, "ciało nie spadło: {pos:?}");
    }

    /// Impuls porusza ciało w poziomie.
    #[test]
    fn impulse_moves_a_body() {
        let mut w = PhysicsWorld::new(Vec2::ZERO);
        let b = w.add_dynamic_box(Vec2::ZERO, Vec2::new(2.0, 2.0));
        w.apply_impulse(&b, Vec2::new(100.0, 0.0), true);
        w.step(1.0 / 60.0);
        assert!(w.velocity(&b).x > 0.0, "impuls nie zadziałał");
    }
}

