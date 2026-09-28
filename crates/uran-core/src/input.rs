//! Stan wejścia: klawiatura, mysz, kółko. Oparty na `winit`, ale z wygodnym
//! API (`just_pressed` zamiast samodzielnego śledzenia eventów).

use std::collections::HashSet;

use uran_math::Vec2;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

/// Klasa klawiszy (alias, żeby nie trzeba było importować `winit` w grze).
pub use winit::keyboard::KeyCode as Key;
/// Przycisk myszy.
pub use winit::event::MouseButton as Mouse;
/// Zbiorczy stan modyfikatorów (Shift/Ctrl/Alt/Super).
pub use winit::keyboard::ModifiersState as Modifiers;

/// Maksymalny ruch myszy raportowany w jednej klatce (ochrona po refocus).
const MAX_MOUSE_DELTA: f32 = 200.0;

/// Migawka wejścia z bieżącej klatki.
#[derive(Debug, Clone, Default)]
pub struct Input {
    keys: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    keys_released: HashSet<KeyCode>,

    buttons: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    buttons_released: HashSet<MouseButton>,

    mouse_position: Vec2,
    mouse_previous: Vec2,
    scroll_delta: Vec2,
    cursor_inside: bool,
    modifiers: ModifiersState,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wywoływane przez `App` dla każdego `WindowEvent`.
    pub fn handle_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                // `physical_key` to `PhysicalKey`; gry chcą widzieć `KeyCode`
                // (niezależny od układu klawiatury), więc wyciągamy wariant kodowy.
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.set_key(code, event.state);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::MouseInput { state, button, .. } => self.set_mouse_button(*button, *state),
            WindowEvent::CursorMoved { position, .. } => {
                self.set_mouse_position(position.x as f32, position.y as f32)
            }
            WindowEvent::CursorLeft { .. } => self.cursor_inside = false,
            WindowEvent::CursorEntered { .. } => self.cursor_inside = true,
            WindowEvent::MouseWheel { delta, .. } => {
                // przeliczenie "pikseli" na linie (~16 px) mieści się
                // w nawyku użytkownika podobnie jak LineDelta
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                    MouseScrollDelta::PixelDelta(p) => (p.x as f32 / 16.0, p.y as f32 / 16.0),
                };
                self.add_scroll(x, y);
            }
            _ => {}
        }
    }

    /// Ustawia stan klawisza (niezależne od winit — łatwiej testować).
    pub fn set_key(&mut self, key: KeyCode, state: ElementState) {
        match state {
            ElementState::Pressed => {
                self.keys.insert(key);
                self.keys_pressed.insert(key);
            }
            ElementState::Released => {
                self.keys.remove(&key);
                self.keys_released.insert(key);
            }
        }
    }

    /// Ustawia stan przycisku myszy.
    pub fn set_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        match state {
            ElementState::Pressed => {
                self.buttons.insert(button);
                self.buttons_pressed.insert(button);
            }
            ElementState::Released => {
                self.buttons.remove(&button);
                self.buttons_released.insert(button);
            }
        }
    }

    /// Przesuwa kursor (w pikselach okna).
    pub fn set_mouse_position(&mut self, x: f32, y: f32) {
        self.mouse_previous = self.mouse_position;
        self.mouse_position = Vec2::new(x, y);
    }

    /// Dodaje przewinięcie kółka (w „liniach").
    pub fn add_scroll(&mut self, x: f32, y: f32) {
        self.scroll_delta += Vec2::new(x, y);
    }

    /// Wywoływane raz na klatkę, po wszystkich systemach.
    pub fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.scroll_delta = Vec2::ZERO;
    }


    // --- Klawiatura ---

    /// Klawisz jest wciśnięty.
    pub fn pressed(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }

    /// Klawisz został właśnie wciśnięty (edge-triggered).
    pub fn just_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    pub fn just_released(&self, key: KeyCode) -> bool {
        self.keys_released.contains(&key)
    }

    /// `any_pressed(&[Key::A, Key::Left])` — wygodne przy aliasach klawiszy.
    pub fn any_pressed(&self, keys: &[KeyCode]) -> bool {
        keys.iter().any(|k| self.pressed(*k))
    }

    pub fn any_just_pressed(&self, keys: &[KeyCode]) -> bool {
        keys.iter().any(|k| self.just_pressed(*k))
    }

    /// `-1.0 / 0.0 / 1.0` z dwóch klawiszy (np. A/D) — do sterowania ruchem.
    pub fn axis(&self, negative: KeyCode, positive: KeyCode) -> f32 {
        f32::from(self.pressed(positive)) - f32::from(self.pressed(negative))
    }

    pub fn shift(&self) -> bool {
        self.modifiers.shift_key()
    }

    pub fn control(&self) -> bool {
        self.modifiers.control_key()
    }

    pub fn alt(&self) -> bool {
        self.modifiers.alt_key()
    }

    // --- Mysz ---

    /// Pozycja kursora w **pikselach okna** (lewy górny róg = 0,0).
    pub fn mouse_position(&self) -> Vec2 {
        self.mouse_position
    }

    /// Ruch myszy od poprzedniej pozycji, limitowany, żeby skok po refocus
    /// nie „teleportował" kamery.
    pub fn mouse_delta(&self) -> Vec2 {
        (self.mouse_position - self.mouse_previous)
            .clamp(Vec2::splat(-MAX_MOUSE_DELTA), Vec2::splat(MAX_MOUSE_DELTA))
    }

    /// Delta kółka myszy w „liniach" (dodatnie Y = przewinięcie w górę).
    pub fn scroll_delta(&self) -> Vec2 {
        self.scroll_delta
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.buttons.contains(&button)
    }

    pub fn mouse_just_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }

    pub fn mouse_just_released(&self, button: MouseButton) -> bool {
        self.buttons_released.contains(&button)
    }

    /// Lewy przycisk — skrót (99% gier używa tylko go).
    pub fn primary_pressed(&self) -> bool {
        self.mouse_pressed(MouseButton::Left)
    }

    pub fn primary_just_pressed(&self) -> bool {
        self.mouse_just_pressed(MouseButton::Left)
    }

    /// Kursor w obrębie okna.
    pub fn cursor_inside(&self) -> bool {
        self.cursor_inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_pressed_just_pressed_released() {
        let mut input = Input::new();
        assert!(!input.pressed(KeyCode::Space));

        input.set_key(KeyCode::Space, ElementState::Pressed);
        assert!(input.pressed(KeyCode::Space));
        assert!(input.just_pressed(KeyCode::Space));
        assert_eq!(input.axis(KeyCode::KeyA, KeyCode::KeyD), 0.0);

        input.end_frame();
        // po end_frame klawisz nadal jest trzymany, ale nie jest "świeży"
        assert!(input.pressed(KeyCode::Space));
        assert!(!input.just_pressed(KeyCode::Space));

        input.set_key(KeyCode::Space, ElementState::Released);
        assert!(!input.pressed(KeyCode::Space));
        assert!(input.just_released(KeyCode::Space));
    }

    #[test]
    fn axis_from_two_keys() {
        let mut input = Input::new();
        input.set_key(KeyCode::KeyD, ElementState::Pressed);
        assert_eq!(input.axis(KeyCode::KeyA, KeyCode::KeyD), 1.0);
        input.set_key(KeyCode::KeyA, ElementState::Pressed);
        assert_eq!(input.axis(KeyCode::KeyA, KeyCode::KeyD), 0.0);
        input.set_key(KeyCode::KeyA, ElementState::Released);
        assert_eq!(input.axis(KeyCode::KeyA, KeyCode::KeyD), 1.0);
        assert!(input.any_pressed(&[KeyCode::KeyW, KeyCode::KeyD]));
        assert!(!input.any_pressed(&[KeyCode::KeyQ]));
        assert!(input.any_just_pressed(&[KeyCode::Escape, KeyCode::KeyD]));
    }

    #[test]
    fn mouse_buttons_and_primary() {
        let mut input = Input::new();
        input.set_mouse_button(MouseButton::Left, ElementState::Pressed);
        assert!(input.primary_pressed());
        assert!(input.primary_just_pressed());
        input.end_frame();
        assert!(!input.primary_just_pressed());
        assert!(input.primary_pressed());

        input.set_mouse_button(MouseButton::Left, ElementState::Released);
        assert!(!input.primary_pressed());
        assert!(input.mouse_just_released(MouseButton::Left));
    }

    #[test]
    fn cursor_tracking_and_delta() {
        let mut input = Input::new();
        input.set_mouse_position(100.0, 50.0);
        assert_eq!(input.mouse_position(), Vec2::new(100.0, 50.0));
        // pierwszy ruch liczymy od (0,0)
        assert_eq!(input.mouse_delta(), Vec2::new(100.0, 50.0));

        input.set_mouse_position(110.0, 50.0);
        assert_eq!(input.mouse_delta(), Vec2::new(10.0, 0.0));
    }

    #[test]
    fn mouse_delta_is_clamped() {
        let mut input = Input::new();
        input.set_mouse_position(10_000.0, 0.0);
        assert!(input.mouse_delta().x <= MAX_MOUSE_DELTA, "skok po refocus musi być ograniczony");
    }

    #[test]
    fn scroll_accumulates_and_resets() {
        let mut input = Input::new();
        input.add_scroll(0.0, 2.0);
        input.add_scroll(0.0, 1.0);
        assert_eq!(input.scroll_delta(), Vec2::new(0.0, 3.0));
        input.end_frame();
        assert_eq!(input.scroll_delta(), Vec2::ZERO);
    }

    #[test]
    fn end_frame_clears_edges_but_keeps_state() {
        let mut input = Input::new();
        input.set_key(KeyCode::KeyW, ElementState::Pressed);
        input.set_mouse_button(MouseButton::Left, ElementState::Pressed);
        input.end_frame();

        assert!(input.pressed(KeyCode::KeyW));
        assert!(input.primary_pressed());
        assert!(!input.just_pressed(KeyCode::KeyW));
        assert!(!input.primary_just_pressed());
    }
}


