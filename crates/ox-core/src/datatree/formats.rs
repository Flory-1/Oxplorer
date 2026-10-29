//! The parsers behind [`super::parse`]: each walks its format and feeds the
//! [`Builder`].

use super::{Builder, Kind, ParseError};

/// Error a visitor returns to stop once the tree is full.
const FULL: &str = "\u{0}full";

// --- JSON ---------------------------------------------------------------

/// JSON, with `//` and `/* */` comments and trailing commas allowed
/// (as in VS Code's settings files).
pub(super) fn json(b: &mut Builder, text: &str) -> Result<(), ParseError> {
    use serde::de::DeserializeSeed;
    let text = strip_jsonc(text);
    let mut de = serde_json::Deserializer::from_str(&text);
    let result = JsonValue { b, key: None }
        .deserialize(&mut de)
        .and_then(|()| de.end());
    match result {
        Ok(()) => Ok(()),
        Err(_) if b.truncated() => Ok(()),
        Err(e) => {
            let message = e.to_string();
            let suffix = format!(" at line {} column {}", e.line(), e.column());
            Err(ParseError {
                message: message.strip_suffix(&suffix).unwrap_or(&message).to_owned(),
                line: e.line(),
                column: e.column(),
            })
        }
    }
}

/// `text` with comments and trailing commas blanked out, so positions in
/// errors stay the same.
pub(super) fn strip_jsonc(text: &str) -> String {
    let mut out = text.as_bytes().to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for b in &mut out[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    };
    let b = text.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    // Where the last comma outside strings is that might be trailing.
    let mut comma: Option<usize> = None;
    while i < b.len() {
        let c = b[i];
        if in_string {
            match c {
                b'\\' => i += 1,
                b'"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_string = true;
                comma = None;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..]
                    .iter()
                    .position(|&c| c == b'\n')
                    .map_or(b.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
                blank(&mut out, i, end);
                i = end;
                continue;
            }
            b',' => comma = Some(i),
            b'}' | b']' => {
                if let Some(at) = comma.take() {
                    out[at] = b' ';
                }
            }
            c if c.is_ascii_whitespace() => {}
            _ => comma = None,
        }
        i += 1;
    }
    // Only ASCII bytes were replaced (by spaces), so this stays UTF-8.
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Takes one JSON value (in document order, unlike `serde_json::Value`).
struct JsonValue<'a> {
    b: &'a mut Builder,
    key: Option<String>,
}

impl JsonValue<'_> {
    fn leaf<E: serde::de::Error>(self, kind: Kind, value: String) -> Result<(), E> {
        match self.b.leaf(kind, self.key, value) {
            true => Ok(()),
            false => Err(E::custom(FULL)),
        }
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for JsonValue<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for JsonValue<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<(), E> {
        self.leaf(Kind::Bool, v.to_string())
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<(), E> {
        self.leaf(Kind::Number, v.to_string())
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<(), E> {
        self.leaf(Kind::Number, v.to_string())
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<(), E> {
        self.leaf(Kind::Number, format!("{v:?}"))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<(), E> {
        self.leaf(Kind::String, v.to_owned())
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<(), E> {
        self.leaf(Kind::String, v)
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        self.leaf(Kind::Null, "null".to_owned())
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        self.visit_unit()
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        use serde::de::Error;
        let b = self.b;
        if !b.open(Kind::Array, self.key) {
            return Err(A::Error::custom(FULL));
        }
        while seq
            .next_element_seed(JsonValue {
                b: &mut *b,
                key: None,
            })?
            .is_some()
        {}
        b.close();
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        use serde::de::Error;
        let b = self.b;
        if !b.open(Kind::Object, self.key) {
            return Err(A::Error::custom(FULL));
        }
        while let Some(key) = map.next_key::<String>()? {
            map.next_value_seed(JsonValue {
                b: &mut *b,
                key: Some(key),
            })?;
        }
        b.close();
        Ok(())
    }
}

// --- XML ----------------------------------------------------------------

/// XML elements with their attributes and text (comments, processing
/// instructions and the declaration are left out).
pub(super) fn xml(b: &mut Builder, text: &str) -> Result<(), ParseError> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    // Text between tags, collected across entity references.
    let mut pending = String::new();
    let flush = |b: &mut Builder, pending: &mut String| {
        let t = pending.trim();
        let ok = t.is_empty() || b.leaf(Kind::Text, None, t.to_owned());
        pending.clear();
        ok
    };
    let error = |reader: &quick_xml::Reader<&[u8]>, e: &dyn std::fmt::Display| {
        ParseError::at(e.to_string(), text, reader.error_position() as usize)
    };
    loop {
        let event = reader.read_event().map_err(|e| error(&reader, &e))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                if !flush(b, &mut pending) || !b.open(Kind::Element, Some(name_of(e.name()))) {
                    return Ok(());
                }
                for attr in e.attributes() {
                    let attr = attr.map_err(|e| error(&reader, &e))?;
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_or_else(|_| attr.value.to_string(), |v| v.into_owned());
                    if !b.leaf(Kind::Attribute, Some(name_of(attr.key)), value) {
                        return Ok(());
                    }
                }
                if empty {
                    b.close();
                }
            }
            Event::End(_) => {
                if !flush(b, &mut pending) {
                    return Ok(());
                }
                b.close();
            }
            Event::Text(t) => pending.push_str(&t.xml10_content()),
            Event::CData(t) => pending.push_str(&t.into_inner()),
            Event::GeneralRef(r) => {
                let name = r.xml10_content();
                match r.resolve_char_ref() {
                    Ok(Some(c)) => pending.push(c),
                    _ => match name.as_ref() {
                        "lt" => pending.push('<'),
                        "gt" => pending.push('>'),
                        "amp" => pending.push('&'),
                        "quot" => pending.push('"'),
                        "apos" => pending.push('\''),
                        // Entities of a DTD stay as written.
                        other => pending.push_str(&format!("&{other};")),
                    },
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if b.open_count() > 0 && !b.truncated() {
        let name = b.innermost().unwrap_or_default().to_owned();
        return Err(ParseError::at(
            crate::trf!("The element <{name}> isn't closed.", name = name),
            text,
            text.len(),
        ));
    }
    Ok(())
}

fn name_of(name: quick_xml::name::QName) -> String {
    name.as_ref().to_owned()
}

// --- YAML ---------------------------------------------------------------

/// YAML; several documents in one file become an array of them.
pub(super) fn yaml(b: &mut Builder, text: &str) -> Result<(), ParseError> {
    let docs = yaml_rust2::YamlLoader::load_from_str(text).map_err(|e| ParseError {
        message: e.info().to_owned(),
        line: e.marker().line(),
        column: e.marker().col() + 1,
    })?;
    match docs.as_slice() {
        [] => {}
        [doc] => {
            yaml_node(b, None, doc);
        }
        docs => {
            if b.open(Kind::Array, None) {
                for doc in docs {
                    if !yaml_node(b, None, doc) {
                        break;
                    }
                }
                b.close();
            }
        }
    }
    Ok(())
}

/// Adds `y`; false once the tree is full.
fn yaml_node(b: &mut Builder, key: Option<String>, y: &yaml_rust2::Yaml) -> bool {
    use yaml_rust2::Yaml;
    match y {
        Yaml::Hash(map) => {
            if !b.open(Kind::Object, key) {
                return false;
            }
            let ok = map.iter().all(|(k, v)| yaml_node(b, Some(yaml_key(k)), v));
            b.close();
            ok
        }
        Yaml::Array(items) => {
            if !b.open(Kind::Array, key) {
                return false;
            }
            let ok = items.iter().all(|v| yaml_node(b, None, v));
            b.close();
            ok
        }
        Yaml::String(s) => b.leaf(Kind::String, key, s.clone()),
        Yaml::Integer(n) => b.leaf(Kind::Number, key, n.to_string()),
        Yaml::Real(s) => b.leaf(Kind::Number, key, s.clone()),
        Yaml::Boolean(v) => b.leaf(Kind::Bool, key, v.to_string()),
        Yaml::Alias(n) => b.leaf(Kind::String, key, format!("*{n}")),
        Yaml::Null | Yaml::BadValue => b.leaf(Kind::Null, key, "null".to_owned()),
    }
}

fn yaml_key(k: &yaml_rust2::Yaml) -> String {
    use yaml_rust2::Yaml;
    match k {
        Yaml::String(s) | Yaml::Real(s) => s.clone(),
        Yaml::Integer(n) => n.to_string(),
        Yaml::Boolean(v) => v.to_string(),
        Yaml::Null => "~".to_owned(),
        _ => "?".to_owned(),
    }
}

// --- TOML ---------------------------------------------------------------

/// TOML, with keys in the order they appear in the file.
pub(super) fn toml(b: &mut Builder, text: &str) -> Result<(), ParseError> {
    let table = toml::de::DeTable::parse(text).map_err(|e| {
        let at = e.span().map_or(0, |s| s.start);
        ParseError::at(e.message().trim(), text, at)
    })?;
    if b.open(Kind::Object, None) {
        toml_table(b, table.get_ref());
        b.close();
    }
    Ok(())
}

fn toml_table(b: &mut Builder, table: &toml::de::DeTable) -> bool {
    let mut entries: Vec<_> = table.iter().collect();
    entries.sort_by_key(|(k, _)| k.span().start);
    entries
        .into_iter()
        .all(|(k, v)| toml_value(b, Some(k.get_ref().to_string()), v.get_ref()))
}

fn toml_value(b: &mut Builder, key: Option<String>, v: &toml::de::DeValue) -> bool {
    use toml::de::DeValue;
    match v {
        DeValue::Table(t) => {
            if !b.open(Kind::Object, key) {
                return false;
            }
            let ok = toml_table(b, t);
            b.close();
            ok
        }
        DeValue::Array(items) => {
            if !b.open(Kind::Array, key) {
                return false;
            }
            let ok = items.iter().all(|v| toml_value(b, None, v.get_ref()));
            b.close();
            ok
        }
        DeValue::String(s) => b.leaf(Kind::String, key, s.to_string()),
        DeValue::Integer(n) => b.leaf(Kind::Number, key, n.to_string()),
        DeValue::Float(n) => b.leaf(Kind::Number, key, n.to_string()),
        DeValue::Boolean(v) => b.leaf(Kind::Bool, key, v.to_string()),
        DeValue::Datetime(d) => b.leaf(Kind::Date, key, d.to_string()),
    }
}
