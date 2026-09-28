//! Generyczny magazyn assetów z uchwytami i generacjami (wzorzec „slot map").

use std::marker::PhantomData;

use crate::handle::Handle;

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Tablica assetów typu `T` z stabilnymi uchwytami.
///
/// Usunięcie zasobu nie przesuwa innych — stare uchwyty po prostu przestają
/// być ważne (ich generacja się nie zgadza).
pub struct Assets<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    _marker: PhantomData<T>,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Assets<T> {
    pub fn new() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), _marker: PhantomData }
    }

    /// Dodaje asset i zwraca uchwyt.
    pub fn insert(&mut self, value: T) -> Handle<T> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            return Handle::new(index, slot.generation);
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot { generation: 0, value: Some(value) });
        Handle::new(index, 0)
    }

    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let slot = self.slots.get(handle.index() as usize)?;
        (slot.generation == handle.generation()).then_some(slot.value.as_ref())?
    }

    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.index() as usize)?;
        (slot.generation == handle.generation()).then_some(slot.value.as_mut())?
    }

    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// Zwalnia slot i unieważnia wszystkie uchwyty, które na niego wskazywały.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let index = handle.index() as usize;
        let slot = self.slots.get_mut(index)?;
        if slot.generation != handle.generation() {
            return None;
        }
        let value = slot.value.take();
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index as u32);
        value
    }

    /// Liczba aktualnie żywych assetów.
    pub fn len(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iteruje po wszystkich assetach wraz z uchwytami.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            slot.value
                .as_ref()
                .map(|v| (Handle::new(i as u32, slot.generation), v))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut assets = Assets::new();
        let a = assets.insert(42u32);
        let b = assets.insert(7u32);
        assert_eq!(*assets.get(a).unwrap(), 42);
        assert_eq!(*assets.get(b).unwrap(), 7);
        assert_eq!(assets.len(), 2);
        assert_ne!(a, b);
    }

    #[test]
    fn handles_are_unique_even_after_remove() {
        let mut assets = Assets::<u32>::new();
        let a = assets.insert(1);
        assets.insert(2);
        assert_eq!(assets.remove(a), Some(1));
        // stary uchwyt jest martwy
        assert_eq!(assets.get(a), None);
        assert!(!assets.contains(a));
        // nowy asset zajmuje zwolniony slot, ale z nową generacją
        let c = assets.insert(3);
        assert_eq!(c.index(), a.index());
        assert_ne!(c.generation(), a.generation());
        assert_eq!(*assets.get(c).unwrap(), 3);
    }

    #[test]
    fn get_mut_and_iter() {
        let mut assets = Assets::new();
        let a = assets.insert(1u8);
        let b = assets.insert(2u8);
        *assets.get_mut(a).unwrap() = 10;
        assert_eq!(*assets.get(a).unwrap(), 10);
        let mut all: Vec<u8> = assets.iter().map(|(_, v)| *v).collect();
        all.sort();
        assert_eq!(all, vec![2, 10]);
        assets.remove(b);
        assert_eq!(assets.iter().count(), 1);
    }

    #[test]
    fn invalid_handle_is_never_valid() {
        let mut assets = Assets::new();
        let h: Handle<u32> = Handle::default();
        assert!(assets.get(h).is_none());
        assert!(assets.remove(h).is_none());
    }
}
