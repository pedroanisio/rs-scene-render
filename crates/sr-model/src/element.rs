//! Uniform, name-based access to every element of the typed model.
//!
//! The generated structs are the fast, statically typed view of a document.
//! Templating needs a second view: overrides, binds and variants address
//! attributes by name (`override target="title" property="size"`), and the
//! evaluator reads the static value of whichever attribute an `animate`
//! names. [`Element`] provides that view for every struct and child enum.

use std::any::Any;
use std::fmt;

use crate::diag::Loc;
use crate::parse::{ParseValue, ValueError};
use crate::values::{Aspect, Color, Fps, LanguageTag, Length, Paint, PaintRef, Point2, Sha256, Timecode};

/// A dynamically typed attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum AttrValue {
    /// Any number (double, integer, checked newtype).
    Num(f64),
    /// Boolean.
    Bool(bool),
    /// String, enumeration literal or id.
    Str(String),
    /// Length with unit.
    Length(Length),
    /// Colour or token reference.
    Color(Color),
    /// Paint.
    Paint(Paint),
    /// `x,y` point.
    Point(Point2),
    /// Number list.
    Numbers(Vec<f64>),
    /// Token list (IDREFS, NMTOKENS).
    Tokens(Vec<String>),
}

impl fmt::Display for AttrValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttrValue::Num(v) => write!(f, "{v}"),
            AttrValue::Bool(v) => write!(f, "{v}"),
            AttrValue::Str(v) => f.write_str(v),
            AttrValue::Length(v) => write!(f, "{v}"),
            AttrValue::Color(v) => write!(f, "{v}"),
            AttrValue::Paint(v) => write!(f, "{v}"),
            AttrValue::Point(v) => write!(f, "{v}"),
            AttrValue::Numbers(v) => {
                let s: Vec<String> = v.iter().map(|x| x.to_string()).collect();
                f.write_str(&s.join(" "))
            }
            AttrValue::Tokens(v) => f.write_str(&v.join(" ")),
        }
    }
}

/// Conversion of a field value into an [`AttrValue`].
pub trait ToAttr {
    /// The dynamic value.
    fn to_attr(&self) -> AttrValue;
}

macro_rules! num_attr {
    ($($t:ty),*) => {$(
        impl ToAttr for $t {
            fn to_attr(&self) -> AttrValue {
                AttrValue::Num(*self as f64)
            }
        }
    )*};
}
num_attr!(f64, i32, i64, u64);

impl ToAttr for bool {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Bool(*self)
    }
}
impl ToAttr for String {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Str(self.clone())
    }
}
impl ToAttr for Vec<String> {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Tokens(self.clone())
    }
}
impl ToAttr for Vec<f64> {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Numbers(self.clone())
    }
}
impl ToAttr for Vec<Color> {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Tokens(self.iter().map(|c| c.to_string()).collect())
    }
}
impl ToAttr for Vec<Point2> {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Tokens(self.iter().map(|p| p.to_string()).collect())
    }
}
impl ToAttr for Length {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Length(*self)
    }
}
impl ToAttr for Color {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Color(self.clone())
    }
}
impl ToAttr for Paint {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Paint(self.clone())
    }
}
impl ToAttr for Point2 {
    fn to_attr(&self) -> AttrValue {
        AttrValue::Point(*self)
    }
}
macro_rules! str_attr {
    ($($t:ty),*) => {$(
        impl ToAttr for $t {
            fn to_attr(&self) -> AttrValue {
                AttrValue::Str(self.to_string())
            }
        }
    )*};
}
str_attr!(PaintRef, Fps, Timecode, Aspect, Sha256, LanguageTag);

/// Why an attribute could not be set by name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetAttrError {
    /// The element declares no attribute of that name.
    #[error("<{element}> has no attribute @{name}")]
    Unknown {
        /// Element or type name.
        element: &'static str,
        /// Attribute name.
        name: String,
    },
    /// The value does not parse as the attribute's type.
    #[error("@{name}: {error}")]
    Invalid {
        /// Attribute name.
        name: String,
        /// Parse error.
        error: ValueError,
    },
}

pub(crate) fn parse_into<T: ParseValue>(name: &str, raw: &str) -> Result<T, SetAttrError> {
    T::parse_value(raw).map_err(|error| SetAttrError::Invalid { name: name.to_string(), error })
}

/// Name-based access to a model element.
pub trait Element {
    /// XML element name when known from context (child enums), otherwise the
    /// XSD complex type name.
    fn element_name(&self) -> &'static str;
    /// Index of the element's complex type in [`crate::xsd::COMPLEX_TYPES`].
    fn xsd_type(&self) -> usize;
    /// Source position.
    fn loc(&self) -> Loc;
    /// `@id`, if declared and present.
    fn element_id(&self) -> Option<&str>;
    /// Value of attribute `name` (after defaults); `None` when absent or undeclared.
    fn get_attr(&self, name: &str) -> Option<AttrValue>;
    /// Sets attribute `name` from its lexical form.
    fn set_attr(&mut self, name: &str, raw: &str) -> Result<(), SetAttrError>;
    /// Text content, for elements with simple content.
    fn text(&self) -> Option<&str> {
        None
    }
    /// Replaces the text content; ignored for element-only content.
    fn set_text(&mut self, _text: String) {}
    /// Visits child elements in document order.
    fn visit<'s>(&'s self, f: &mut dyn FnMut(&'s dyn Element));
    /// Visits child elements mutably in document order.
    fn visit_mut(&mut self, f: &mut dyn FnMut(&mut dyn Element));
    /// Downcasting support.
    fn as_any(&self) -> &dyn Any;
    /// True when the complex type declares attribute `name`.
    fn declares(&self, name: &str) -> bool {
        crate::xsd::COMPLEX_TYPES[self.xsd_type()].attr(name).is_some()
    }
}

/// Visits `e` and every descendant, depth first, in document order.
pub fn walk<'s>(e: &'s dyn Element, f: &mut dyn FnMut(&'s dyn Element)) {
    f(e);
    e.visit(&mut |c| walk(c, f));
}

/// Child elements of `e`, collected.
pub fn children(e: &dyn Element) -> Vec<&dyn Element> {
    let mut v = Vec::new();
    e.visit(&mut |c| v.push(c));
    v
}

/// Mutable depth-first walk over `e` and its descendants.
pub fn walk_mut(e: &mut dyn Element, f: &mut dyn FnMut(&mut dyn Element)) {
    f(e);
    e.visit_mut(&mut |c| walk_mut(c, f));
}
