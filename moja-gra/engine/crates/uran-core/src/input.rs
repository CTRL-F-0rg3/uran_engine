//! Stan wejścia: klawiatura, mysz, kółko. Oparty na `winit`, ale z wygodnym
//! API (`just_pressed` zamiast samodzielnego śledzenia eventów).

use std::collections::HashSet;

use uran_math::Vec2;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

/// Przycisk myszy.
pub use winit::event::MouseButton as Mouse;
/// Klasa klawiszy (alias, żeby nie trzeba było importować `winit` w grze).
pub use winit::keyboard::KeyCode as Key;
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
    /// Ruch myszy zgłoszony przez system przy **zablokowanym** kursorze.
    ///
    /// `CursorMoved` mówi tylko, *gdzie* jest kursor. Przy blokadzie
    /// kursor stoi w centrum okna i nie zmiera pozycji, więc
    /// `mouse_position - mouse_previous` zawsze wynosi ZERO i
    /// obrót kamery zamiera. System raportuje wtedy ruch względny
    /// (`DeviceEvent::MouseMotion`), który nie ma pozycji — dlatego
    /// trzymamy go osobno i sumujemy co klatkę.
    ///
    /// Właśnie dlatego blokada kursora psuła sterowanie: winit nie
    /// wysyła `DeviceEvent::MouseMotion` sam z siebie do
    /// `ApplicationHandler` jako `WindowEvent`, więc trzeba go
    /// obsłużyć osobno w `device_event` (patrz `App`).
    raw_motion: Vec2,
    /// Czy kursor został już zlokalizowany przynajmniej raz.
    ///
    /// Bez tego pierwsze zdarzenie `CursorMoved` po starcie dawałoby
    /// deltę równą pozycji kursora (skok z (0,0) na środek okna) —
    /// patrz [`Input::set_mouse_position`].
    mouse_seen: bool,
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
    ///
    /// Pierwsze zdarzenie po otwarciu okna **nie** generuje delty.
    /// Kursor startuje w (0,0), a system raportuje od razu jego
    /// pozycję w środku okna — bez tego zabezpieczenia
    /// [`Input::mouse_delta`] zwróciłby jednorazowy „ruch" o pół
    /// ekranu i obrót kamery przeskoczyłby o ~90°. Limit
    /// `MAX_MOUSE_DELTA` tego nie łapie, bo 640 px mieści się
    /// w limicie 200 px dopiero po przeskalowaniu, a przeskok jest
    /// pojedynczy i zbyt duży, by uznać go za ruch użytkownika.
    ///
    /// Dlatego pierwszą pozycję zapamiętujemy jako poprzednią i zwracamy
    /// zerową deltę; kolejne zdarzenia już liczą się normalnie.
    pub fn set_mouse_position(&mut self, x: f32, y: f32) {
        if !self.mouse_seen {
            // Pierwsza pozycja to punkt odniesienia, nie ruch.
            self.mouse_seen = true;
            self.mouse_position = Vec2::new(x, y);
            self.mouse_previous = self.mouse_position;
            return;
        }
        self.mouse_previous = self.mouse_position;
        self.mouse_position = Vec2::new(x, y);
    }

    /// Dodaje przewinięcie kółka (w „liniach").
    pub fn add_scroll(&mut self, x: f32, y: f32) {
        self.scroll_delta += Vec2::new(x, y);
    }

    /// Dodaje ruch względny myszy (z `DeviceEvent::MouseMotion`).
    ///
    /// Używane przy zablokowanym kursorze, gdzie pozycja w oknie
    /// stoi w miejscu i nie nadaje się do liczenia delty. Ruch
    /// **sumujemy**, bo zdarzeń bywa wiele na klatkę — wtedy mysz
    /// szybciej by się obracała niż przy jednym zdarzeniu na klatkę.
    pub fn add_raw_motion(&mut self, x: f32, y: f32) {
        self.raw_motion += Vec2::new(x, y);
    }

    /// Wyzerowuje zgromadzony ruch względny.
    ///
    /// Wywoływane w [`Input::end_frame`] — inaczej ruch z całej sesji
    /// sumowałby się w nieskończoność i kamera skoczyłaby raz, a potem
    /// już nigdy by nie reagowała.
    pub fn clear_raw_motion(&mut self) {
        self.raw_motion = Vec2::ZERO;
    }

    /// Wywoływane raz na klatkę, po wszystkich systemach.
    pub fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.buttons_released.clear();
        self.scroll_delta = Vec2::ZERO;
        self.raw_motion = Vec2::ZERO;
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
    ///
    /// Przy **zablokowanym** kursorze pozycja w oknie nie zmienia się,
    /// więc liczenie z `mouse_position` dawałoby wieczne ZERO i kamera
    /// stałaby nieruchomo. Dlatego jeśli system podał ruch względny
    /// (`DeviceEvent::MouseMotion`), bierzemy właśnie jego — on jest
    /// jedynym źródłem obrotu kamery w trybie blokady.
    ///
    /// Fallback na pozycję zostaje dla trybu bez blokady oraz na
    /// platformach, które nie dają ruchu względnego.
    pub fn mouse_delta(&self) -> Vec2 {
        let limit = Vec2::splat(MAX_MOUSE_DELTA);
        if self.raw_motion != Vec2::ZERO {
            // Ruch względny sumujemy z pozycją: przy blokadzie pozycja
            // stoi, ale gdyby ją ktoś jednak poruszył, nie wolno
            // zgubić ani jednego ze źródeł.
            (self.raw_motion + (self.mouse_position - self.mouse_previous)).clamp(-limit, limit)
        } else {
            (self.mouse_position - self.mouse_previous).clamp(-limit, limit)
        }
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
        // Pierwsza pozycja to punkt odniesienia, nie ruch — patrz
        // `first_mouse_position_is_not_treated_as_movement`.
        assert_eq!(input.mouse_delta(), Vec2::ZERO);

        input.set_mouse_position(110.0, 50.0);
        assert_eq!(input.mouse_delta(), Vec2::new(10.0, 0.0));
    }

    #[test]
    fn mouse_delta_is_clamped() {
        let mut input = Input::new();
        // Dwa zdarzenia: pierwsze ustala pozycję odniesienia, drugie
        // symuluje skok kursora (np. po powrocie z alt-tab).
        input.set_mouse_position(0.0, 0.0);
        input.set_mouse_position(10_000.0, 0.0);
        assert!(
            input.mouse_delta().x <= MAX_MOUSE_DELTA,
            "skok po refocus musi być ograniczony"
        );
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
    fn first_mouse_position_is_not_treated_as_movement() {
        // Regres: przy starcie okna kursor przeskakuje z (0,0) na środek.
        // Ta pierwsza pozycja to punkt odniesienia, a nie ruch — inaczej
        // obrót kamery przeskakiwałby o pół ekranu przy starcie gry.
        let mut input = Input::new();
        input.set_mouse_position(640.0, 360.0);
        assert_eq!(
            input.mouse_delta(),
            Vec2::ZERO,
            "pierwszy ruch to nie delta"
        );

        // Drugie zdarzenie już jest prawdziwym ruchem.
        input.set_mouse_position(660.0, 360.0);
        assert_eq!(input.mouse_delta(), Vec2::new(20.0, 0.0));

        // Delta jest różnicą, więc cofanie działa symetrycznie.
        input.set_mouse_position(650.0, 355.0);
        assert_eq!(input.mouse_delta(), Vec2::new(-10.0, -5.0));
    }

    /// Regres: zablokowany kursor nie może zatrzymać kamery.
    ///
    /// Przy `CursorGrabMode::Locked` kursor stoi w centrum okna, więc
    /// pozycja się nie zmienia i `mouse_delta()` liczona z niej
    /// zawsze wynosi ZERO. Jedynym źródłem obrotu jest wtedy
    /// `DeviceEvent::MouseMotion` (ruch względny). Ten test udowadnia,
    /// że sam ruch względny wystarczy do obrócenia kamery.
    #[test]
    fn raw_motion_turns_the_camera_while_cursor_is_locked() {
        let mut input = Input::new();

        // Kursor zablokowany: system wciąż raportuje tę samą
        // pozycję w centrum okna, więc z niej nie da się wyczytać ruchu.
        input.set_mouse_position(640.0, 360.0);
        assert_eq!(
            input.mouse_delta(),
            Vec2::ZERO,
            "pozycja sama w sobie nie niesie informacji o ruchu"
        );

        // Ruch względny 12 px w prawo — kamera musi się obrócić.
        input.add_raw_motion(12.0, 0.0);
        assert_eq!(
            input.mouse_delta(),
            Vec2::new(12.0, 0.0),
            "zablokowany kursor musi dawać obrót z ruchu względnego"
        );
    }

    /// Ruch względny musi się sumować w klatce i zerować po niej.
    ///
    /// Bez `end_frame` suma rośłaby w nieskończoność: kamera skoczyłaby
    /// raz, a potem już nigdy by nie reagowała.
    #[test]
    fn raw_motion_accumulates_and_resets_each_frame() {
        let mut input = Input::new();
        input.add_raw_motion(3.0, 1.0);
        input.add_raw_motion(4.0, 0.0);
        assert_eq!(
            input.mouse_delta(),
            Vec2::new(7.0, 1.0),
            "zdarzenia na jednej klatce sumują się"
        );

        input.end_frame();
        assert_eq!(
            input.mouse_delta(),
            Vec2::ZERO,
            "po klatce akumulator musi być czysty"
        );
    }

    /// Ruch względny podlega temu samemu limitowi co pozycja.
    #[test]
    fn raw_motion_is_clamped_too() {
        let mut input = Input::new();
        input.add_raw_motion(10_000.0, 0.0);
        assert!(
            input.mouse_delta().x <= MAX_MOUSE_DELTA,
            "skok z surowego wejścia musi być ograniczony tak samo"
        );
    }

    /// Bez blokady kursora zachowanie się nie zmienia — pozycja
    /// nadal wystarcza (np. `uran-tanks`, `junak-rider`).
    #[test]
    fn unlocked_cursor_still_uses_position_delta() {
        let mut input = Input::new();
        input.set_mouse_position(100.0, 50.0);
        input.set_mouse_position(110.0, 50.0);
        assert_eq!(
            input.mouse_delta(),
            Vec2::new(10.0, 0.0),
            "try bez blokady nie może polegać na ruchu względnym"
        );
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
