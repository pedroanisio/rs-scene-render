//! Conversion of XML lexical values into typed Rust values.
//!
//! Every attribute type in the model implements [`ParseValue`]. Values reach
//! these parsers only after structural validation, so a parse failure here
//! means the validator and the model disagree — an internal error reported as
//! [`ModelError`].

use roxmltree::Node;

use crate::diag::Loc;
use crate::xsd::simple::collapse;
use crate::xsd::{parse_xsd_double, parse_xsd_integer};

/// A lexical value that does not denote a value of the target type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValueError(pub String);

impl ValueError {
    /// A value error with a message.
    pub fn new(msg: impl Into<String>) -> Self {
        ValueError(msg.into())
    }
}

/// Parses an XML lexical value.
pub trait ParseValue: Sized {
    /// Parses `s`. Implementations apply the XSD whitespace rule of their type.
    fn parse_value(s: &str) -> Result<Self, ValueError>;
}

impl ParseValue for String {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        Ok(s.to_string())
    }
}

impl ParseValue for f64 {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        parse_xsd_double(s.trim()).ok_or_else(|| ValueError::new(format!("{s:?} is not a number")))
    }
}

impl ParseValue for bool {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        match s.trim() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(ValueError::new(format!("{s:?} is not a boolean"))),
        }
    }
}

macro_rules! int_parse {
    ($($t:ty),*) => {$(
        impl ParseValue for $t {
            fn parse_value(s: &str) -> Result<Self, ValueError> {
                parse_xsd_integer(s.trim())
                    .and_then(|v| <$t>::try_from(v).ok())
                    .ok_or_else(|| ValueError::new(format!("{s:?} is not a valid {}", stringify!($t))))
            }
        }
    )*};
}
int_parse!(i32, i64, u64);

impl ParseValue for Vec<String> {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        Ok(collapse(s).split(' ').filter(|t| !t.is_empty()).map(str::to_string).collect())
    }
}

impl ParseValue for Vec<f64> {
    fn parse_value(s: &str) -> Result<Self, ValueError> {
        collapse(s).split(' ').filter(|t| !t.is_empty()).map(f64::parse_value).collect()
    }
}

/// Error raised while building the typed model from a validated document.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{loc}: {message}")]
pub struct ModelError {
    /// What went wrong.
    pub message: String,
    /// Where.
    pub loc: Loc,
}

impl ModelError {
    pub(crate) fn unexpected(n: Node<'_, '_>) -> Self {
        ModelError { message: format!("unexpected element <{}>", n.tag_name().name()), loc: Loc::of(n) }
    }

    pub(crate) fn missing(n: Node<'_, '_>, child: &str) -> Self {
        ModelError { message: format!("<{}> lacks required <{child}>", n.tag_name().name()), loc: Loc::of(n) }
    }

    fn value(n: Node<'_, '_>, attr: &str, e: ValueError) -> Self {
        let loc = n
            .attributes()
            .find(|a| a.name() == attr && a.namespace().is_none())
            .map(|a| Loc::at(a.range().start))
            .unwrap_or_else(|| Loc::of(n));
        ModelError { message: format!("@{attr} of <{}>: {e}", n.tag_name().name()), loc }
    }
}

/// Required attribute.
pub(crate) fn req<T: ParseValue>(n: Node<'_, '_>, name: &str) -> Result<T, ModelError> {
    match n.attribute(name) {
        Some(v) => T::parse_value(v).map_err(|e| ModelError::value(n, name, e)),
        None => {
            Err(ModelError { message: format!("<{}> lacks required @{name}", n.tag_name().name()), loc: Loc::of(n) })
        }
    }
}

/// Optional attribute without a default.
pub(crate) fn opt<T: ParseValue>(n: Node<'_, '_>, name: &str) -> Result<Option<T>, ModelError> {
    n.attribute(name).map(|v| T::parse_value(v).map_err(|e| ModelError::value(n, name, e))).transpose()
}

/// Present attribute value.
pub(crate) fn attr<T: ParseValue>(n: Node<'_, '_>, name: &str, v: &str) -> Result<T, ModelError> {
    T::parse_value(v).map_err(|e| ModelError::value(n, name, e))
}

/// Optional attribute with a schema default. The default is parsed once per
/// call site and cloned afterwards.
macro_rules! def {
    ($n:expr, $name:expr, $default:expr, $t:ty) => {
        match $n.attribute($name) {
            Some(v) => $crate::parse::attr::<$t>($n, $name, v),
            None => {
                static DEFAULT: std::sync::OnceLock<$t> = std::sync::OnceLock::new();
                Ok::<$t, ModelError>(
                    DEFAULT
                        .get_or_init(|| {
                            <$t as ParseValue>::parse_value($default).expect("schema default is a valid value")
                        })
                        .clone(),
                )
            }
        }
    };
}
pub(crate) use def;

/// Concatenated text content of an element with simple content.
pub(crate) fn text_content(n: Node<'_, '_>) -> String {
    n.children().filter(|c| c.is_text()).filter_map(|c| c.text()).collect()
}
