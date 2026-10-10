//! Nazwy skryptów-szablonów generowanych w `data/` elementu.

/// Wszystkie identyfikatory skryptów: `a1`..`a9`, `b1`..`b9`, ... `z1`..`z9`.
///
/// Każdy z nich to pusty szablon, który gracz wypełnia funkcjami (animacje,
/// efekty, dźwięki) i który jest spięty z `object.rs` / `init.rs` elementu.
pub fn script_ids() -> Vec<String> {
    let mut ids = Vec::with_capacity(26 * 9);
    for letter in b'a'..=b'z' {
        for digit in 1..=9 {
            ids.push(format!("{}{}", letter as char, digit));
        }
    }
    ids
}
