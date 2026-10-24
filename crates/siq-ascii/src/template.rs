//! A dialect's templates: a line with fields in it, such as `MA {target:steps}`.
//!
//! A template renders a line from values: the commands the driver sends, and the replies the
//! emulator sends. It also matches a line back into values, through the anchored regex it
//! compiles, with a named group per field.

use std::collections::HashMap;

use regex::Regex;

/// How a field is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A length in whole steps of the dialect's step, or a speed in steps per second.
    Steps,
    /// A length in µm, or a speed in µm/s, with this many decimals.
    Um(u8),
    /// A length in mm, or a speed in mm/s, with this many decimals.
    Mm(u8),
    /// A whole number, such as an error code.
    Int,
    /// `1` or `0`.
    Flag,
    /// Any text, such as a model name.
    Text,
}

impl Kind {
    /// What it holds, as the dialect's checks say it.
    pub fn class(self) -> &'static str {
        match self {
            Kind::Steps | Kind::Um(_) | Kind::Mm(_) => "a length",
            Kind::Int => "a number",
            Kind::Flag => "a flag",
            Kind::Text => "text",
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        let decimals = |default: u8, rest: &str| match rest.strip_prefix('.') {
            None if rest.is_empty() => Ok(default),
            Some(digits) => digits.parse().ok().filter(|&d: &u8| d <= 9).ok_or(()),
            None => Err(()),
        };
        let kind = match text {
            "steps" => Ok(Kind::Steps),
            "int" => Ok(Kind::Int),
            "flag" => Ok(Kind::Flag),
            "text" => Ok(Kind::Text),
            _ if text.starts_with("um") => decimals(3, &text[2..]).map(Kind::Um),
            _ if text.starts_with("mm") => decimals(4, &text[2..]).map(Kind::Mm),
            _ => Err(()),
        };
        let known = "steps, um, um.N, mm, mm.N, int, flag or text";
        kind.map_err(|()| format!("`{text}` isn't a kind of field: write {known}"))
    }

    fn pattern(self) -> &'static str {
        match self {
            Kind::Steps | Kind::Int => r"[-+]?\d+",
            Kind::Um(_) | Kind::Mm(_) => r"[-+]?\d+(?:\.\d+)?",
            Kind::Flag => "[01]",
            Kind::Text => ".+?",
        }
    }
}

/// A field's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A length in µm, or a speed in µm/s.
    Length(f64),
    /// A whole number.
    Int(i64),
    /// A flag.
    Flag(bool),
    /// Text.
    Text(String),
}

/// Fields' values, by name.
pub type Values = HashMap<String, Value>;

#[derive(Debug, Clone)]
enum Part {
    Text(String),
    Field(String, Kind),
}

/// A parsed template. It deserializes from its text.
#[derive(Debug, Clone)]
pub struct Template {
    parts: Vec<Part>,
    regex: Regex,
}

impl Template {
    /// Parses `source`: text, with fields `{name:kind}`. Braces only delimit fields.
    pub fn parse(source: &str) -> Result<Self, String> {
        let (mut parts, mut pattern, mut rest) = (Vec::new(), String::from("^"), source);
        let text = |parts: &mut Vec<Part>, pattern: &mut String, text: &str| {
            if !text.is_empty() {
                pattern.push_str(&regex::escape(text));
                parts.push(Part::Text(text.to_owned()));
            }
        };
        while let Some(open) = rest.find(['{', '}']) {
            text(&mut parts, &mut pattern, &rest[..open]);
            let field = rest[open..]
                .strip_prefix('{')
                .ok_or("a `}` without its `{`")?;
            let (field, after) = field.split_once('}').ok_or("a `{` without its `}`")?;
            let (name, kind) = field
                .split_once(':')
                .ok_or_else(|| format!("{{{field}}} has no kind: write {{{field}:steps}}, say"))?;
            let named =
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
            if !named {
                return Err(format!("`{name}` isn't a field's name: use a-z and _"));
            }
            if parts
                .iter()
                .any(|part| matches!(part, Part::Field(n, _) if n == name))
            {
                return Err(format!("{{{name}}} is there twice"));
            }
            let kind = Kind::parse(kind)?;
            pattern.push_str(&format!("(?P<{name}>{})", kind.pattern()));
            parts.push(Part::Field(name.to_owned(), kind));
            rest = after;
        }
        text(&mut parts, &mut pattern, rest);
        pattern.push('$');
        let regex = Regex::new(&pattern).map_err(|error| error.to_string())?;
        Ok(Self { parts, regex })
    }

    /// Its fields, by name and kind.
    pub fn fields(&self) -> impl Iterator<Item = (&str, Kind)> {
        self.parts.iter().filter_map(|part| match part {
            Part::Field(name, kind) => Some((name.as_str(), *kind)),
            Part::Text(_) => None,
        })
    }

    /// Whether it has a field `name`.
    pub fn has(&self, name: &str) -> bool {
        self.fields().any(|(field, _)| field == name)
    }

    /// The line, with each field written from `values`, and `step` µm per step. A field without
    /// a value of its kind is left empty.
    pub fn render(&self, values: &Values, step: f64) -> String {
        let mut line = String::new();
        for part in &self.parts {
            match part {
                Part::Text(text) => line.push_str(text),
                Part::Field(name, kind) => line.push_str(&match (kind, values.get(name)) {
                    (Kind::Steps, Some(Value::Length(v))) => {
                        format!("{}", (v / step).round() as i64)
                    }
                    (Kind::Um(n), Some(Value::Length(v))) => format!("{v:.*}", usize::from(*n)),
                    (Kind::Mm(n), Some(Value::Length(v))) => {
                        format!("{:.*}", usize::from(*n), v / 1e3)
                    }
                    (Kind::Int, Some(Value::Int(i))) => i.to_string(),
                    (Kind::Flag, Some(Value::Flag(f))) => u8::from(*f).to_string(),
                    (Kind::Text, Some(Value::Text(t))) => t.clone(),
                    _ => String::new(),
                }),
            }
        }
        line
    }

    /// The values in `line`, if it matches, with `step` µm per step.
    pub fn matches(&self, line: &str, step: f64) -> Option<Values> {
        let captures = self.regex.captures(line)?;
        let mut values = Values::new();
        for (name, kind) in self.fields() {
            let text = captures.name(name)?.as_str();
            let value = match kind {
                Kind::Steps => Value::Length(text.parse::<i64>().ok()? as f64 * step),
                Kind::Um(_) => Value::Length(text.parse().ok()?),
                Kind::Mm(_) => Value::Length(text.parse::<f64>().ok()? * 1e3),
                Kind::Int => Value::Int(text.parse().ok()?),
                Kind::Flag => Value::Flag(text == "1"),
                Kind::Text => Value::Text(text.to_owned()),
            };
            values.insert(name.to_owned(), value);
        }
        Some(values)
    }
}

impl<'de> serde::Deserialize<'de> for Template {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let source = String::deserialize(deserializer)?;
        Self::parse(&source).map_err(serde::de::Error::custom)
    }
}
