//! Data sources for templating: JSON, CSV and TSV rows, and list parameters.

use std::collections::BTreeMap;
use std::sync::Arc;

use sr_model::model::DataSourceFormat;

use crate::expr::vm::V;

fn from_json(j: &serde_json::Value) -> V {
    match j {
        serde_json::Value::Null => V::Undef,
        serde_json::Value::Bool(b) => V::Bool(*b),
        serde_json::Value::Number(n) => V::Num(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => V::Str(s.as_str().into()),
        serde_json::Value::Array(a) => V::Arr(a.iter().map(from_json).collect()),
        serde_json::Value::Object(o) => V::Obj(Arc::new(o.iter().map(|(k, v)| (k.clone(), from_json(v))).collect())),
    }
}

/// A CSV/TSV cell: numbers become numbers, everything else stays text.
fn cell(s: &str) -> V {
    let t = s.trim();
    if !t.is_empty() {
        if let Some(n) = sr_model::xsd::parse_xsd_double(t).filter(|n| n.is_finite()) {
            return V::Num(n);
        }
    }
    V::Str(s.into())
}

/// Splits delimited text into records (RFC 4180 quoting). Plain blank lines
/// are skipped; explicitly quoted empty records are retained.
pub fn split_records(text: &str, delim: char) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut record_started = false;
    let mut chars = text.chars().peekable();
    let mut line = 1;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
            record_started = true;
        } else if c == delim {
            record_started = true;
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            line += 1;
            row.push(std::mem::take(&mut field));
            if record_started {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
            record_started = false;
        } else {
            record_started = true;
            field.push(c);
        }
    }
    if quoted {
        return Err(format!("unterminated quoted field (line {line})"));
    }
    if record_started {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

/// Parses a data source into rows. JSON accepts an array of records or
/// values, or an object with a `rows` array; CSV and TSV need a header row.
pub fn parse(format: DataSourceFormat, text: &str) -> Result<Vec<V>, String> {
    match format {
        DataSourceFormat::Json => {
            let j: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
            match &j {
                serde_json::Value::Array(a) => Ok(a.iter().map(from_json).collect()),
                serde_json::Value::Object(o) => match o.get("rows") {
                    Some(serde_json::Value::Array(a)) => Ok(a.iter().map(from_json).collect()),
                    _ => Err("a JSON data source is an array of rows or an object with a \"rows\" array".into()),
                },
                _ => Err("a JSON data source is an array of rows".into()),
            }
        }
        DataSourceFormat::Csv | DataSourceFormat::Tsv => {
            let delim = if format == DataSourceFormat::Csv { ',' } else { '\t' };
            let mut recs = split_records(text, delim)?.into_iter();
            let Some(header) = recs.next() else { return Ok(Vec::new()) };
            let header: Vec<String> = header.into_iter().map(|h| h.trim().to_string()).collect();
            recs.enumerate()
                .map(|(i, r)| {
                    if r.len() != header.len() {
                        return Err(format!("row {} has {} fields; the header has {}", i + 1, r.len(), header.len()));
                    }
                    let m: BTreeMap<String, V> = header.iter().cloned().zip(r.iter().map(|c| cell(c))).collect();
                    Ok(V::Obj(Arc::new(m)))
                })
                .collect()
        }
    }
}

/// Parses a `type="list"` parameter value: a JSON array, or comma-separated items.
pub fn parse_list(s: &str) -> V {
    let t = s.trim();
    if t.starts_with('[') {
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(t) {
            return from_json(&j);
        }
    }
    if t.is_empty() {
        return V::Arr(Arc::from(Vec::new()));
    }
    V::Arr(t.split(',').map(cell).map(|v| if let V::Str(s) = v { V::Str(s.trim().into()) } else { v }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_empty_records_survive_line_endings_and_eof() {
        for ending in ["", "\n", "\r\n", "\r"] {
            let text = format!("name\n\"\"{ending}");
            assert_eq!(split_records(&text, ',').unwrap(), vec![vec!["name".to_string()], vec![String::new()]]);
            for format in [DataSourceFormat::Csv, DataSourceFormat::Tsv] {
                let rows = parse(format, &text).unwrap();
                assert_eq!(rows.len(), 1);
                let V::Obj(row) = &rows[0] else { panic!("expected data row") };
                assert_eq!(row["name"], V::Str("".into()));
            }
        }
        assert_eq!(split_records("\"\"", ',').unwrap(), vec![vec![String::new()]]);
        assert_eq!(split_records("\n\"\"\n\n\"\"\n", ',').unwrap(), vec![vec![String::new()], vec![String::new()]]);
        assert_eq!(
            split_records("\n\r\n\r", ',').unwrap(),
            Vec::<Vec<String>>::new(),
            "plain blank lines remain skipped"
        );
        assert_eq!(split_records("\"\",\"\"", ',').unwrap(), vec![vec![String::new(), String::new()]]);
    }

    #[test]
    fn formats() {
        let rows = parse(DataSourceFormat::Json, r#"[{"name":"Ada","price":9}]"#).unwrap();
        let V::Obj(r) = &rows[0] else { panic!() };
        assert_eq!(r["price"], V::Num(9.0));
        let rows = parse(DataSourceFormat::Csv, "name,price\n\"Lovelace, Ada\",9\r\nLinus,\"1\"\"2\"\n").unwrap();
        assert_eq!(rows.len(), 2);
        let V::Obj(r) = &rows[0] else { panic!() };
        assert_eq!(r["name"], V::Str("Lovelace, Ada".into()));
        let V::Obj(r) = &rows[1] else { panic!() };
        assert_eq!(r["price"], V::Str("1\"2".into()));
        assert_eq!(parse(DataSourceFormat::Tsv, "a\tb\n1\t2\n").unwrap().len(), 1);
        assert!(parse(DataSourceFormat::Csv, "a,b\n1\n").unwrap_err().contains("row 1"));
        assert_eq!(parse_list("a, b ,3"), V::Arr(vec![V::Str("a".into()), V::Str("b".into()), V::Num(3.0)].into()));
        assert_eq!(parse_list("[1,2]"), V::nums(&[1.0, 2.0]));
    }
}
