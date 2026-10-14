//! Lexical Windows path normalization for allow-list comparison.
//!
//! Requests reach the policy as canonical paths (the service resolves them with
//! `GetFinalPathNameByHandleW`). Allow-list entries are written by people, so
//! both sides go through the same lexical rules before they are compared.

/// An absolute Windows path split into a root and components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalPath {
    /// `C:` or `\\server\share`.
    pub root: String,
    pub components: Vec<String>,
}

impl NormalPath {
    /// Display form: `C:\a\b`, `C:\` or `\\server\share\a`.
    pub fn to_path_string(&self) -> String {
        let mut out = self.root.clone();
        if self.components.is_empty() {
            out.push('\\');
        }
        for c in &self.components {
            out.push('\\');
            out.push_str(c);
        }
        out
    }

    /// Parent folder, or `None` for a root.
    pub fn parent(&self) -> Option<NormalPath> {
        let mut parent = self.clone();
        parent.components.pop()?;
        Some(parent)
    }
}

/// Normalizes an absolute path. Returns `None` for relative, drive-relative,
/// device (`\\.\`) and malformed UNC paths.
///
/// * `/` is treated as `\`; `\\?\` and `\\?\UNC\` prefixes are removed.
/// * `.` is dropped and `..` removes the previous component (never the root).
/// * Trailing dots and spaces are trimmed from each component, as Win32 does.
///
/// Case is preserved; compare with [`fold_case`].
pub fn normalize(path: &str) -> Option<NormalPath> {
    let path = path.replace('/', "\\");
    let path = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path
    };

    let (root, rest) = if let Some(unc) = path.strip_prefix(r"\\") {
        if unc.starts_with(['.', '?']) {
            return None;
        }
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().filter(|s| !s.is_empty())?;
        let share = parts.next().filter(|s| !s.is_empty())?;
        (format!(r"\\{server}\{share}"), parts.next().unwrap_or(""))
    } else {
        let mut chars = path.chars();
        let drive = chars.next().filter(char::is_ascii_alphabetic)?;
        if chars.next() != Some(':') || chars.next() != Some('\\') {
            return None;
        }
        (format!("{}:", drive.to_ascii_uppercase()), &path[3..])
    };

    let mut components: Vec<String> = Vec::new();
    for raw in rest.split('\\') {
        match raw {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            _ => {
                let trimmed = raw.trim_end_matches(['.', ' ']);
                if !trimmed.is_empty() {
                    components.push(trimmed.to_string());
                }
            }
        }
    }
    Some(NormalPath { root, components })
}

/// Case folding for comparisons: simple one-to-one uppercase mapping per
/// character, close to the NTFS upcase table.
pub fn fold_case(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(p: &str) -> Option<String> {
        normalize(p).map(|n| n.to_path_string())
    }

    #[test]
    fn drives_and_unc() {
        assert_eq!(
            norm(r"c:\Windows\System32\").as_deref(),
            Some(r"C:\Windows\System32")
        );
        assert_eq!(norm("C:/a//b/./c").as_deref(), Some(r"C:\a\b\c"));
        assert_eq!(norm(r"C:\").as_deref(), Some(r"C:\"));
        assert_eq!(
            norm(r"\\srv\share\x\..\y").as_deref(),
            Some(r"\\srv\share\y")
        );
        assert_eq!(norm(r"\\?\C:\a\b").as_deref(), Some(r"C:\a\b"));
        assert_eq!(
            norm(r"\\?\UNC\srv\share\a").as_deref(),
            Some(r"\\srv\share\a")
        );
    }

    #[test]
    fn dot_dot_cannot_escape_root() {
        assert_eq!(
            norm(r"C:\a\..\..\..\Windows").as_deref(),
            Some(r"C:\Windows")
        );
        assert_eq!(
            norm(r"\\srv\share\..\..\other").as_deref(),
            Some(r"\\srv\share\other")
        );
    }

    #[test]
    fn win32_trailing_dots_and_spaces() {
        assert_eq!(
            norm(r"C:\Tools.\app.exe. ").as_deref(),
            Some(r"C:\Tools\app.exe")
        );
        assert_eq!(norm(r"C:\a\...\b").as_deref(), Some(r"C:\a\b"));
    }

    #[test]
    fn rejects_relative_and_devices() {
        for p in [
            "app.exe",
            r"C:app.exe",
            r"\Windows",
            r"\\.\pipe\x",
            r"\\srv",
            r"\\srv\",
            r"1:\x",
            "",
        ] {
            assert_eq!(norm(p), None, "{p}");
        }
    }

    #[test]
    fn parent_and_case() {
        let p = normalize(r"C:\a\b.exe").unwrap();
        assert_eq!(p.parent().unwrap().to_path_string(), r"C:\a");
        assert_eq!(normalize(r"C:\").unwrap().parent(), None);
        assert_eq!(fold_case("c:\\Straße\\ä"), "C:\\STRAßE\\Ä");
    }
}
