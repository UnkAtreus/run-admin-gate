//! Windows command-line tokenizing, following the Microsoft C runtime rules
//! (the same rules as `CommandLineToArgvW` for arguments after the first).

/// Splits the first argument off `line`.
///
/// Returns the unquoted argument and the rest of the line, starting right
/// after it (leading whitespace not removed). `program_name` selects the
/// simpler rules used for `argv[0]`: quotes delimit, backslashes are literal.
pub fn split_arg(line: &str, program_name: bool) -> Option<(String, &str)> {
    let line = line.trim_start_matches([' ', '\t']);
    if line.is_empty() {
        return None;
    }
    if program_name {
        return Some(match line.strip_prefix('"') {
            Some(rest) => match rest.find('"') {
                Some(end) => (rest[..end].to_string(), &rest[end + 1..]),
                None => (rest.to_string(), ""),
            },
            None => {
                let end = line.find([' ', '\t']).unwrap_or(line.len());
                (line[..end].to_string(), &line[end..])
            }
        });
    }

    let bytes: Vec<(usize, char)> = line.char_indices().collect();
    let mut out = String::new();
    let mut in_quotes = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i].1;
        match c {
            '\\' => {
                let start = i;
                while i < bytes.len() && bytes[i].1 == '\\' {
                    i += 1;
                }
                let count = i - start;
                if i < bytes.len() && bytes[i].1 == '"' {
                    out.extend(std::iter::repeat('\\').take(count / 2));
                    if count % 2 == 1 {
                        out.push('"');
                        i += 1;
                    }
                } else {
                    out.extend(std::iter::repeat('\\').take(count));
                }
                continue;
            }
            '"' => {
                if in_quotes && i + 1 < bytes.len() && bytes[i + 1].1 == '"' {
                    out.push('"');
                    i += 2;
                    continue;
                }
                in_quotes = !in_quotes;
            }
            ' ' | '\t' if !in_quotes => {
                return Some((out, &line[bytes[i].0..]));
            }
            _ => out.push(c),
        }
        i += 1;
    }
    Some((out, ""))
}

/// Splits a whole command line into arguments.
pub fn split_all(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    let mut first = true;
    while let Some((arg, tail)) = split_arg(rest, first) {
        out.push(arg);
        rest = tail;
        first = false;
    }
    out
}

/// Quotes one argument so [`split_arg`] reads it back unchanged.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
                continue;
            }
            _ => {}
        }
        if c != '\\' {
            out.extend(std::iter::repeat('\\').take(backslashes));
            backslashes = 0;
            out.push(c);
        }
    }
    out.extend(std::iter::repeat('\\').take(backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name_rules() {
        assert_eq!(
            split_arg(r#""C:\a b\x.exe" rest"#, true),
            Some((r"C:\a b\x.exe".into(), " rest"))
        );
        assert_eq!(
            split_arg(r"C:\x.exe  /a", true),
            Some((r"C:\x.exe".into(), "  /a"))
        );
        assert_eq!(
            split_arg(r#"C:\a\"b c"#, true),
            Some((r#"C:\a\"b"#.into(), " c"))
        );
        assert_eq!(split_arg("   ", true), None);
    }

    #[test]
    fn argument_rules() {
        // Cases from the Microsoft "Parsing C command-line arguments" table.
        assert_eq!(split_all(r#"x "a b c" d e"#)[1..], ["a b c", "d", "e"]);
        assert_eq!(
            split_all(r#"x "ab\"c" "\\" d"#)[1..],
            [r#"ab"c"#, r"\", "d"]
        );
        assert_eq!(
            split_all(r#"x a\\\b d"e f"g h"#)[1..],
            [r"a\\\b", "de fg", "h"]
        );
        assert_eq!(split_all(r#"x a\\\"b c d"#)[1..], [r#"a\"b"#, "c", "d"]);
        assert_eq!(split_all(r#"x a\\\\"b c" d e"#)[1..], [r"a\\b c", "d", "e"]);
        assert_eq!(split_all(r#"x "a""b""#)[1..], [r#"a"b"#]);
    }

    #[test]
    fn rest_is_kept_verbatim() {
        let line = r#""C:\rag.exe" /quiet "C:\Program Files\app.exe" -x "y z""#;
        let (_, rest) = split_arg(line, true).unwrap();
        let (flag, rest) = split_arg(rest, false).unwrap();
        let (program, rest) = split_arg(rest, false).unwrap();
        assert_eq!(
            (flag.as_str(), program.as_str()),
            ("/quiet", r"C:\Program Files\app.exe")
        );
        assert_eq!(rest.trim_start(), r#"-x "y z""#);
    }

    #[test]
    fn quoting_round_trips() {
        for arg in [
            "plain",
            "with space",
            r#"q"uote"#,
            r"trail\",
            r"C:\a b\",
            "",
            r#"\\""#,
            "tab\there",
        ] {
            let line = format!("x {}", quote_arg(arg));
            assert_eq!(split_all(&line)[1], arg, "{line}");
        }
    }
}
