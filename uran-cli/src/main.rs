//! `uran` — narzędzie do zarządzania projektami i elementami gier na silniku Uran.
//!
//! Polecenia:
//! * `uran new <nazwa> [--engine <ścieżka>] [--at <katalog>]` — nowy projekt
//!   (jak `cargo new --bin`) z silnikiem skopiowanym ze wskazanej ścieżki,
//! * `uran element <nazwa> [--at <katalog>]` — nowy element gry
//!   (`src/elements/<nazwa>/` z `object.rs`, `init.rs` i skryptami `data/a1..z9`).

mod element;
mod project;
mod scripts;
mod templates;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("błąd: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return Ok(());
    }
    match args[0].as_str() {
        "new" => cmd_new(&args[1..]),
        "element" => cmd_element(&args[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "--version" | "-V" => {
            println!("uran {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(format!("nieznane polecenie `{other}` (spróbuj `uran help`)")),
    }
}

fn cmd_new(args: &[String]) -> Result<(), String> {
    let mut name = None;
    let mut engine: Option<PathBuf> = None;
    let mut at = PathBuf::from(".");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bin" => i += 1, // binarny typ to domyślny — flaga dla zgodności z `cargo new`
            "--engine" | "-e" => {
                engine = Some(PathBuf::from(require_value(args, &mut i)?));
                i += 1;
            }
            "--at" | "--path" => {
                at = PathBuf::from(require_value(args, &mut i)?);
                i += 1;
            }
            "--help" | "-h" => {
                print_new_help();
                return Ok(());
            }
            flag if flag.starts_with('-') => return Err(format!("nieznana flaga `{flag}`")),
            positional => {
                if name.is_some() {
                    return Err("podano dwie nazwy projektu".to_string());
                }
                name = Some(positional.to_string());
                i += 1;
            }
        }
    }

    let name = name.ok_or_else(|| "brak nazwy projektu (użycie: `uran new <nazwa> [--engine <ścieżka>]`)".to_string())?;

    let root = project::scaffold(&project::Scaffold { name, engine, at })?;
    println!("✅ utworzono projekt `{}`", root.display());
    println!("   wejdź:    cd {}", root.display());
    println!("   uruchom:  cargo run");
    Ok(())
}

fn cmd_element(args: &[String]) -> Result<(), String> {
    let mut name = None;
    let mut at = PathBuf::from(".");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--at" | "--path" => {
                at = PathBuf::from(require_value(args, &mut i)?);
                i += 1;
            }
            "--help" | "-h" => {
                print_element_help();
                return Ok(());
            }
            flag if flag.starts_with('-') => return Err(format!("nieznana flaga `{flag}`")),
            positional => {
                if name.is_some() {
                    return Err("podano dwie nazwy elementu".to_string());
                }
                name = Some(positional.to_string());
                i += 1;
            }
        }
    }

    let name = name.ok_or_else(|| "brak nazwy elementu (użycie: `uran element <nazwa>`)".to_string())?;

    element::create_element(&at, &name)?;
    println!(
        "✅ utworzono element `{name}` w `{}`",
        at.join("src/elements").join(&name).display()
    );
    Ok(())
}

/// Zwraca wartość następnego argumentu (dla flag `--x <wartość>`).
fn require_value(args: &[String], i: &mut usize) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("flaga `{}` wymaga wartości", args[*i - 1]))
}

fn print_help() {
    println!(
        "uran {} — zarządzanie projektami i elementami silnika Uran\n\n\
         Użycie:\n  \
         uran new <nazwa> [--engine <ścieżka>] [--at <katalog>] [--bin]\n  \
         uran element <nazwa> [--at <katalog>]\n  \
         uran help | --version\n\n\
         Polecenia:\n  \
         new       nowy projekt gry (jak `cargo new --bin`) + kopia silnika\n  \
         element   nowy element gry w `src/elements/<nazwa>/`\n\n\
         Flagi `new`:\n  \
         --engine   ścieżka do katalogu silnika (workspace z `uran-engine/` i `crates/`)\n  \
         --at       katalog, w którym utworzyć projekt (domyślnie bieżący)\n  \
         --bin      typ binarny (domyślny, zachowany dla zgodności z `cargo new`)",
        env!("CARGO_PKG_VERSION")
    );
}

fn print_new_help() {
    println!(
        "uran new <nazwa> [--engine <ścieżka>] [--at <katalog>] [--bin]\n\n\
         Tworzy nowy projekt gry. Działanie:\n  \
         * zakłada katalog `<nazwa>/` z `Cargo.toml`, `src/` i `assets/`,\n  \
         * kopiuje silnik z `--engine` do `<nazwa>/engine/`,\n  \
         * generuje `src/main.rs`, `src/object.rs`, `src/init.rs` i `src/elements/`.\n\n\
         Przykład:\n  \
         uran new moja-gra --engine ../uran_engine"
    );
}

fn print_element_help() {
    println!(
        "uran element <nazwa> [--at <katalog>]\n\n\
         Tworzy element gry w `src/elements/<nazwa>/`:\n  \
         * `object.rs` i `init.rs` — główne pliki elementu,\n  \
         * `data/a1.rs` .. `data/z9.rs` — puste szablony skryptów,\n  \
         * dopina element do `src/elements/mod.rs`, `src/object.rs` i `src/init.rs`.\n\n\
         Przykład (w katalogu projektu):\n  \
         uran element przeciwnik"
    );
}
