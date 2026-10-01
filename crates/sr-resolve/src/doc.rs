//! Editing one attribute of one element in a document's text, leaving every
//! other byte (comments, layout, quoting) as it was.

/// Byte range of the start tag `<name … id="id" …>` (or `/>`).
fn find_tag(text: &str, name: &str, id: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let open = format!("<{name}");
    let mut from = 0;
    while let Some(off) = text[from..].find('<') {
        let start = from + off;
        let rest = &text[start..];
        // comments, CDATA sections and processing instructions hold no elements
        let hidden =
            [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")].into_iter().find(|(o, _)| rest.starts_with(o));
        if let Some((o, close)) = hidden {
            from = start + o.len() + rest[o.len()..].find(close)? + close.len();
            continue;
        }
        if !rest.starts_with(&open) {
            from = start + 1;
            continue;
        }
        from = start + open.len();
        // the name must end here
        match bytes.get(from) {
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>') => {}
            _ => continue,
        }
        // the tag's end, skipping quoted values
        let mut i = from;
        let mut quote = None;
        while i < bytes.len() {
            match (quote, bytes[i]) {
                (None, b'"' | b'\'') => quote = Some(bytes[i]),
                (Some(q), c) if c == q => quote = None,
                (None, b'>') => break,
                _ => {}
            }
            i += 1;
        }
        let end = i + 1;
        if attr_span(&text[start..end.min(text.len())], "id")
            .is_some_and(|(_, v0, v1)| &text[start + v0..start + v1] == id)
        {
            return Some((start, end));
        }
    }
    None
}

/// In a start tag: (start of the whitespace before `name=`, value start, value end) of
/// attribute `name`, found by reading the attributes in order (so text inside other values
/// never matches).
fn attr_span(tag: &str, name: &str) -> Option<(usize, usize, usize)> {
    let b = tag.as_bytes();
    // skip `<` and the element name
    let mut i = 1;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    loop {
        let ws = i;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let n0 = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'/' | b'>') {
            i += 1;
        }
        if i == n0 {
            return None;
        }
        let n1 = i;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            return None;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let q = *b.get(i)?;
        if q != b'"' && q != b'\'' {
            return None;
        }
        let v0 = i + 1;
        let v1 = v0 + tag[v0..].find(q as char)?;
        if &tag[n0..n1] == name {
            return Some((ws, v0, v1));
        }
        i = v1 + 1;
    }
}

/// Sets `attr` on the `<name id="id">` start tag, adding it when absent.
pub fn set_attr(text: &str, name: &str, id: &str, attr: &str, value: &str) -> Result<String, String> {
    let (start, end) =
        find_tag(text, name, id).ok_or_else(|| format!("<{name} id=\"{id}\"> not found in the document text"))?;
    let tag = &text[start..end];
    let new_tag = match attr_span(tag, attr) {
        Some((_, v0, v1)) => format!("{}{}{}", &tag[..v0], value, &tag[v1..]),
        None => {
            let close = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len() - 1 };
            let body = tag[..close].trim_end();
            format!("{body} {attr}=\"{value}\"{}", &tag[close..])
        }
    };
    Ok(format!("{}{}{}", &text[..start], new_tag, &text[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_only_the_attribute() {
        let t = "<!-- keep --><assets>\n  <generated  id='a' kind=\"speech\"\n     cacheSha256=\"00\" cache=\"a.wav\"/>\n  <generated id=\"b\" cacheSha256=\"11\"/></assets>";
        let u = set_attr(t, "generated", "a", "cacheSha256", "ff").unwrap();
        assert_eq!(u, t.replacen("cacheSha256=\"00\"", "cacheSha256=\"ff\"", 1));
        let v = set_attr(t, "generated", "b", "cacheSha256", "ee").unwrap();
        assert!(v.contains("id=\"b\" cacheSha256=\"ee\"/>") && v.contains("cacheSha256=\"00\""));
        let w = set_attr(
            "<captionTrack id=\"c\" language=\"en\">\n<cue/></captionTrack>",
            "captionTrack",
            "c",
            "cacheSha256",
            "aa",
        )
        .unwrap();
        assert_eq!(w, "<captionTrack id=\"c\" language=\"en\" cacheSha256=\"aa\">\n<cue/></captionTrack>");
        // a longer element name sharing the prefix, and an id inside another attribute's value
        let x = "<generatedX id=\"a\"/><generated prompt=\"say id='a' k='no'\" id=\"a\"/>";
        let y = set_attr(x, "generated", "a", "k", "v").unwrap();
        assert_eq!(y, "<generatedX id=\"a\"/><generated prompt=\"say id='a' k='no'\" id=\"a\" k=\"v\"/>");
        assert!(set_attr(t, "generated", "zz", "k", "v").is_err());
    }

    #[test]
    fn tags_in_comments_cdata_and_instructions_are_not_elements() {
        let real = r#"<generated id="a" cacheSha256="00"/>"#;
        for decoy in [
            r#"<!-- <generated id="a" cacheSha256="00"/> -->"#,
            r#"<note><![CDATA[<generated id="a" cacheSha256="00"/>]]></note>"#,
            r#"<?note <generated id="a" cacheSha256="00"/> ?>"#,
            r#"<!-- > <generated id="a" cacheSha256="00"/>"#,
        ] {
            let t = format!("<scene>{decoy}{real}</scene>");
            let u = set_attr(&t, "generated", "a", "cacheSha256", "ff");
            if decoy.ends_with("-->") || !decoy.starts_with("<!--") {
                assert_eq!(u.unwrap(), format!("<scene>{decoy}{}</scene>", real.replace("00", "ff")), "{decoy}");
            } else {
                // an unterminated comment hides the rest of the text
                assert!(u.is_err(), "{decoy}");
            }
        }
        assert!(set_attr(r#"<!-- <generated id="a"/> -->"#, "generated", "a", "k", "v").is_err());
    }
}
