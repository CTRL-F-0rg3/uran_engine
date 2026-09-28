//! Uchwyt na assety — lekki, `Copy`, z numerem generacji.
//!
//! `Handle<T>` sam w sobie nie niesie danych; trzyma je [`crate::server::Assets<T>`].
//! Numer generacji chroni przed „use after free": po usunięciu zasobu stary
//! uchwyt przestaje być ważny, zamiast wskazywać na cudzy asset.

use std::fmt;
use std::marker::PhantomData;

/// Uchwyt na asset typu `T` (np. [`crate::loader::Image`]).
///
/// Jest `Copy`, więc można go trzymać w komponentach ECS:
/// ```ignore
/// #[derive(Clone, Copy)]
/// struct Player { texture: Handle<Image> }
/// ```
// Derive NIE wystarcza: wygenerowałby ograniczenia `T: Copy`/`T: PartialEq`,
// których uchwyt nie potrzebuje. `PhantomData<fn() -> T>` jest zawsze Copy
// i zawsze porównywalne, więc implementujemy ręcznie.
pub struct Handle<T> {
    index: u32,
    generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> PartialOrd for Handle<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Handle<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}

impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> Handle<T> {
    /// Tworzy uchwyt z surowych pól. Normalnie uchwyty pochodzą z
    /// [`crate::server::Assets`]; ten konstruktor jest potrzebny głównie
    /// w testach i narzędziach.
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation, _marker: PhantomData }
    }

    /// Odtwarza uchwyt z typowego `HandleId` (np. przy lookupie po kluczu
    /// tekstury w rendererze).
    pub const fn from_id(id: HandleId) -> Self {
        Self { index: id.index, generation: id.generation, _marker: PhantomData }
    }

    /// Indeks w tablicy assetów (przydatne do debugowania / wskaźników).
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Numer generacji — rośnie z każdym zwolnieniem slotu.
    pub const fn generation(self) -> u32 {
        self.generation
    }

    /// „Pusty" uchwyt — nigdy nie wskazuje na istniejący asset.
    pub const fn invalid() -> Self {
        Self { index: u32::MAX, generation: u32::MAX, _marker: PhantomData }
    }
}

impl<T> Default for Handle<T> {
    fn default() -> Self {
        Self::invalid()
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({}v{})", self.index, self.generation)
    }
}

impl<T> fmt::Display for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.index, self.generation)
    }
}

/// Klucz do porównywania uchwytów różnych typów (np. w mapie „która tekstura
/// jest już wczytana") bez łamania `Hash` o `T`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandleId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl<T> From<Handle<T>> for HandleId {
    fn from(h: Handle<T>) -> Self {
        Self { index: h.index, generation: h.generation }
    }
}

impl HandleId {
    /// Tworzy klucz z surowych pól (głównie w testach i narzędziach).
    pub const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Indeks w magazynie assetów.
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Numer generacji.
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl<T> From<HandleId> for Handle<T> {
    fn from(id: HandleId) -> Self {
        Self::from_id(id)
    }
}

impl fmt::Debug for HandleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HandleId({}v{})", self.index, self.generation)
    }
}
