//! The parameters of one file call, read the way the platform reads them.
//!
//! Phoenix hands its file actions one map: the query string's pairs, then the JSON body's fields
//! on top of them. A caller may therefore name the path in either place, and the routes that
//! carry their arguments in the query (a read, a listing, a removal) take them nowhere else.
//! Query values are text, decoded as `application/x-www-form-urlencoded` decodes them; body
//! values keep their JSON type, so a field that must be a JSON boolean is told apart from the
//! text `"true"` exactly as the platform tells them apart.

use serde_json::{Map, Value};

/// Why the parameters could not be read at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParamsRefused {
    /// The query string is not valid percent-encoded UTF-8 text.
    Query,
    /// The body is present and is not one JSON object.
    Body,
}

/// One call's parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Params(Map<String, Value>);

impl Params {
    /// Reads the query string and the body into one map, the body's fields winning.
    ///
    /// # Errors
    ///
    /// Returns which half could not be read.
    pub(crate) fn read(query: Option<&str>, body: &[u8]) -> Result<Self, ParamsRefused> {
        let mut fields = Map::new();
        for pair in query
            .unwrap_or("")
            .split('&')
            .filter(|pair| !pair.is_empty())
        {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let name = decode(name).ok_or(ParamsRefused::Query)?;
            let value = decode(value).ok_or(ParamsRefused::Query)?;
            fields.insert(name, Value::String(value));
        }
        if body.iter().any(|byte| !byte.is_ascii_whitespace()) {
            match serde_json::from_slice::<Value>(body) {
                Ok(Value::Object(object)) => fields.extend(object),
                _ => return Err(ParamsRefused::Body),
            }
        }
        Ok(Self(fields))
    }

    /// The raw value of one field.
    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    /// One field when it is non-empty text, as the platform's `is_binary(x) and x != ""` reads it.
    pub(crate) fn text(&self, name: &str) -> Option<&str> {
        match self.0.get(name) {
            Some(Value::String(text)) if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    /// Whether a field is present at all, whatever its value.
    pub(crate) fn has(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// Whether a flag is set: a JSON `true`, or the text `true` or `1` from a query string.
    pub(crate) fn flag(&self, name: &str) -> bool {
        match self.0.get(name) {
            Some(Value::Bool(set)) => *set,
            Some(Value::String(text)) => text == "true" || text == "1",
            _ => false,
        }
    }

    /// A non-negative whole number, given as a JSON integer or as decimal text.
    ///
    /// `None` is a field that is absent; `Some(Err(()))` is one that is present and is not such a
    /// number, which the caller refuses rather than ignores.
    pub(crate) fn count(&self, name: &str) -> Option<Result<u64, ()>> {
        self.0.get(name).map(|value| match value {
            Value::Number(number) => number.as_u64().ok_or(()),
            Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                text.parse().map_err(|_| ())
            }
            _ => Err(()),
        })
    }
}

/// Decodes one query component: `+` is a space and `%XX` is the byte it names.
fn decode(component: &str) -> Option<String> {
    let bytes = component.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let high = hex(*bytes.get(index + 1)?)?;
                let low = hex(*bytes.get(index + 2)?)?;
                decoded.push((high << 4) | low);
                index += 2;
            }
            other => decoded.push(other),
        }
        index += 1;
    }
    String::from_utf8(decoded).ok()
}

const fn hex(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Params, ParamsRefused};

    #[test]
    fn query_pairs_are_form_decoded() {
        let params =
            Params::read(Some("path=%2Fhome%2Fuser%2Fa+b.txt&x=%C3%A9"), b"").expect("read");
        assert_eq!(params.text("path"), Some("/home/user/a b.txt"));
        assert_eq!(params.text("x"), Some("\u{e9}"));
    }

    #[test]
    fn the_body_wins_over_the_query() {
        let params =
            Params::read(Some("path=/tmp/q"), br#"{"path":"/tmp/b","n":2}"#).expect("read");
        assert_eq!(params.text("path"), Some("/tmp/b"));
        assert_eq!(params.count("n"), Some(Ok(2)));
    }

    #[test]
    fn a_malformed_query_or_body_is_refused() {
        for query in ["path=%2", "path=%zz", "path=%FF"] {
            assert_eq!(
                Params::read(Some(query), b""),
                Err(ParamsRefused::Query),
                "{query}"
            );
        }
        for body in [&b"[1]"[..], b"not json", b"\"text\""] {
            assert_eq!(Params::read(None, body), Err(ParamsRefused::Body));
        }
        assert!(
            Params::read(None, b"  \n").is_ok(),
            "a blank body is no body"
        );
    }

    #[test]
    fn flags_and_counts_read_both_json_and_query_forms() {
        let params = Params::read(
            Some("a=true&b=1&c=false&n=12&bad=-1"),
            br#"{"d":true,"e":"yes"}"#,
        )
        .expect("read");
        assert!(params.flag("a") && params.flag("b") && params.flag("d"));
        assert!(!params.flag("c") && !params.flag("e") && !params.flag("absent"));
        assert_eq!(params.count("n"), Some(Ok(12)));
        assert_eq!(params.count("bad"), Some(Err(())));
        assert_eq!(params.count("absent"), None);
        assert_eq!(params.get("d"), Some(&json!(true)));
    }

    #[test]
    fn empty_text_is_not_text() {
        let params = Params::read(Some("path="), b"").expect("read");
        assert!(params.has("path"));
        assert_eq!(params.text("path"), None);
    }
}
