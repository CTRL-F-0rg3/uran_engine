//! Minimalny parser JSON dla importu glTF.
//!
//! ## Dlaczego własny, a nie `serde_json`
//!
//! Workspace nie zależy od `serde` — w tym środowisku sieć jest
//! zablokowana, więc dorzucanie zależności to ryzyko, że build przestanie
//! się odtwarzać. JSON w glTF jest prosty: obiekty, tablice, liczby,
//! napisy, `true`/`false`/`null`. Nie potrzebujemy tu niczego więcej, a
//! parser ma ~200 linii i zero zależności.
//!
//! ## Czego świadomie nie robimy
//!
//! Nie implementujemy składania par `\uD83D\uDE00` w emoji. glTF używa
//! escapów wyłącznie w nazwach, a tam nie występują. `Json::Str` zwraca
//! wtedy znak zastępczy; gdyby kiedyś to przestało wystarczać, obsługa
//! par surrogate to jedyne miejsce do poprawy.

use std::collections::BTreeMap;
use std::fmt;

/// Wartość JSON — wystarczający podzbiór do odczytu pliku glTF.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Kolejność kluczy nie jest tu potrzebna, więc `BTreeMap` wystarcza
    /// i daje wygodne porównywanie w testach.
    Obj(BTreeMap<String, Json>),
}

impl Json {
    /// Pole obiektu po nazwie; `None` gdy go nie ma lub to nie obiekt.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }

    /// Pole z wartością domyślną — najczęstszy przypadek w glTF, gdzie
    /// brak pola znaczy „zachowaj resztę”.
    pub fn field(&self, key: &str, default: Json) -> Json {
        self.get(key).cloned().unwrap_or(default)
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.as_f64().map(|n| n as u32)
    }

    pub fn as_i32(&self) -> Option<i32> {
        self.as_f64().map(|n| n as i32)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }

    /// Tablica liczb jako `Vec<f64>`; `None` gdy to nie tablica.
    pub fn as_f64_vec(&self) -> Option<Vec<f64>> {
        Some(self.as_array()?.iter().filter_map(Json::as_f64).collect())
    }

    /// Element tablicy — używane przy `nodes`, `meshes`, `skins`.
    pub fn idx(&self, i: usize) -> Option<&Json> {
        self.as_array()?.get(i)
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> ParseError {
        ParseError {
            at: self.i,
            msg: msg.to_string(),
        }
    }

    /// Pomija biały znak. W glTF bywa go dużo — eksporter formatuje
    /// dokument czytelnie, a my i tak czytamy bajty.
    fn ws(&mut self) {
        while let Some(c) = self.b.get(self.i) {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), ParseError> {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("oczekiwano `{}`", c as char)))
        }
    }

    fn lit(&mut self, s: &str, v: Json) -> Result<Json, ParseError> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Ok(v)
        } else {
            Err(self.err("nieznany literał"))
        }
    }

    fn value(&mut self) -> Result<Json, ParseError> {
        match self.b.get(self.i) {
            None => Err(self.err("nieskończony koniec")),
            Some(b'n') => self.lit("null", Json::Null),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'"') => self.string().map(Json::Str),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(_) => self.number(),
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let c = *self
                .b
                .get(self.i)
                .ok_or_else(|| self.err("niezamknięty napis"))?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = *self
                        .b
                        .get(self.i)
                        .ok_or_else(|| self.err("ucięty escape"))?;
                    self.i += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hex = self
                                .b
                                .get(self.i..self.i + 4)
                                .ok_or_else(|| self.err("ucięty \\u"))?;
                            let s =
                                std::str::from_utf8(hex).map_err(|_| self.err("zły hex w \\u"))?;
                            let cp = u32::from_str_radix(s, 16)
                                .map_err(|_| self.err("zły hex w \\u"))?;
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            self.i += 4;
                        }
                        other => out.push(other as char),
                    }
                }
                _ => out.push(c as char),
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        while let Some(c) = self.b.get(self.i) {
            if c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-') {
                self.i += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| self.err("zły UTF-8 w liczbie"))?;
        s.parse::<f64>().map(Json::Num).map_err(|_| ParseError {
            at: start,
            msg: format!("zła liczba `{s}`"),
        })
    }

    fn array(&mut self) -> Result<Json, ParseError> {
        self.eat(b'[')?;
        let mut v = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Json::Arr(v));
        }
        loop {
            self.ws();
            v.push(self.value()?);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Arr(v));
                }
                _ => return Err(self.err("brak `,` lub `]` w tablicy")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, ParseError> {
        self.eat(b'{')?;
        let mut m = BTreeMap::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Json::Obj(m));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            self.eat(b':')?;
            self.ws();
            let v = self.value()?;
            m.insert(k, v);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                _ => return Err(self.err("brak `,` lub `}` w obiekcie")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(s: &str) -> Json {
        parse(s).unwrap_or_else(|e| panic!("nie sparsowano `{s}`: {e}"))
    }

    #[test]
    fn czyta_scalars() {
        assert_eq!(j("1"), Json::Num(1.0));
        assert_eq!(j("-2.5"), Json::Num(-2.5));
        assert_eq!(j("true"), Json::Bool(true));
        assert_eq!(j("null"), Json::Null);
        assert_eq!(j("\"tekst\""), Json::Str("tekst".into()));
    }

    #[test]
    fn czyta_wykladniki() {
        // Accessory glTF zapisują małe liczby jako `1e-2`.
        assert_eq!(j("1e-2").as_f64(), Some(0.01));
        assert_eq!(j("-1.5E3").as_f64(), Some(-1500.0));
    }

    #[test]
    fn czyta_tablicy() {
        assert_eq!(j("[1,2,3]").as_f64_vec(), Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(j("[]").as_array().map(|a| a.len()), Some(0));
    }

    #[test]
    fn czyta_zagniezdzone_obiektow() {
        let v = j(r#"{"a":{"b":[1,{"c":2}]},"d":true}"#);
        assert_eq!(v.get("d").and_then(Json::as_bool), Some(true));
        let inner = v.get("a").and_then(|a| a.get("b")).unwrap();
        assert_eq!(inner.idx(0).and_then(Json::as_f64), Some(1.0));
        assert_eq!(
            inner.idx(1).and_then(|o| o.get("c")).and_then(Json::as_f64),
            Some(2.0)
        );
    }

    #[test]
    fn pole_z_wartoscia_domyslna() {
        let v = j(r#"{"a":1}"#);
        assert_eq!(v.field("a", Json::Null).as_f64(), Some(1.0));
        // brakujące pole dostaje wartość domyślną, nie `None`
        assert_eq!(v.field("b", Json::Num(7.0)).as_f64(), Some(7.0));
    }

    #[test]
    fn odporne_na_biale_znaki_i_escapy() {
        let v = j("  { \"a\" : [ 1 , 2 ] ,\n\t\"s\" : \"x\\ty\" }  ");
        assert_eq!(v.get("a").unwrap().as_f64_vec(), Some(vec![1.0, 2.0]));
        assert_eq!(v.get("s").and_then(Json::as_str), Some("x\ty"));
    }

    #[test]
    fn zglasza_blad_z_pozycja() {
        let e = parse("{").unwrap_err();
        // komunikat musi mówić GDZIE, bo glTF to jeden długi wiersz
        assert!(format!("{e}").contains("bajcie"), "komunikat: {e}");
        assert!(parse("[1,]").is_err());
        assert!(parse("{} smiec").is_err());
        assert!(parse("\"niezamkniete").is_err());
    }
}

/// Błąd z pozycją — bez niej komunikat jest bezużyteczny, bo JSON
/// w glTF bywa jednym wierszem długości 90 kB.
#[derive(Debug)]
pub struct ParseError {
    pub at: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "błąd JSON przy bajcie {}: {}", self.at, self.msg)
    }
}

impl std::error::Error for ParseError {}

/// Parsuje cały dokument.
pub fn parse(src: &str) -> Result<Json, ParseError> {
    let b = src.as_bytes();
    let mut p = Parser { b, i: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != b.len() {
        return Err(p.err("śmieci za wartością"));
    }
    Ok(v)
}
