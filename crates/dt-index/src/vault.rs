use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::warn;
use walkdir::WalkDir;

/// Where a file reference came from — determines trust level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRefSource {
    /// From frontmatter file-registry — high trust, explicit path
    Frontmatter,
    /// From backtick content in note body — needs validation
    InlineCode,
}

/// A reference to a code file found in a vault note.
#[derive(Debug, Clone)]
pub struct FileRef {
    pub path: PathBuf,
    pub line: Option<u32>,
    pub source: FileRefSource,
    /// True if this is a bare filename (e.g. "Foo.java") rather than a path with directories
    pub is_bare_filename: bool,
}

/// Parsed representation of a single vault note.
#[derive(Debug, Clone)]
pub struct VaultNote {
    pub path: PathBuf,
    pub title: String,
    pub note_type: String,
    pub frontmatter: Frontmatter,
    pub file_refs: Vec<FileRef>,
    pub wikilinks: Vec<String>,
    pub summary: String,
    pub body: String,
}

/// YAML frontmatter from a doctrack note.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Frontmatter {
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    pub tags: Vec<String>,
    #[serde(rename = "file-registry")]
    pub file_registry: Vec<String>,
    pub files: Vec<String>,
    pub components: Vec<String>,
    pub related: Vec<String>,
    pub last_updated: Option<String>,
    pub status: Option<String>,
}

/// Parse all markdown notes in a vault directory.
pub fn parse_vault(vault_root: &Path) -> Result<Vec<VaultNote>> {
    let mut notes = Vec::new();

    for entry in WalkDir::new(vault_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match parse_note(path) {
                Ok(note) => notes.push(note),
                Err(e) => warn!("failed to parse note {:?}: {}", path, e),
            }
        }
    }

    Ok(notes)
}

/// Parse a single vault note from a markdown file.
pub fn parse_note(path: &Path) -> Result<VaultNote> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading note: {}", path.display()))?;

    let (frontmatter, body) = split_frontmatter(&content);

    let fm: Frontmatter = if let Some(yaml) = frontmatter {
        serde_yaml::from_str(yaml).unwrap_or_default()
    } else {
        Frontmatter::default()
    };

    let title = fm
        .title
        .clone()
        .or_else(|| extract_h1(&body))
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

    let note_type = fm.note_type.clone().unwrap_or_default();
    let file_refs = extract_file_refs(&fm, &body);
    let wikilinks = extract_wikilinks(&body);
    let summary = extract_summary(&body);

    Ok(VaultNote {
        path: path.to_path_buf(),
        title,
        note_type,
        frontmatter: fm,
        file_refs,
        wikilinks,
        summary,
        body: body.to_string(),
    })
}

/// Split frontmatter from body. Returns (Some(yaml), body) or (None, full_content).
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    if !content.starts_with("---") {
        return (None, content);
    }

    if let Some(end) = content[3..].find("\n---") {
        let yaml = &content[3..3 + end];
        let body = &content[3 + end + 4..];
        (Some(yaml.trim()), body)
    } else {
        (None, content)
    }
}

/// Extract the first H1 heading from markdown body.
fn extract_h1(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return Some(heading.to_string());
        }
    }
    None
}

/// Extract file references from frontmatter file-registry and inline code paths.
fn extract_file_refs(fm: &Frontmatter, body: &str) -> Vec<FileRef> {
    let mut refs = Vec::new();

    // From frontmatter file-registry and files — trusted, always include
    for entry in fm.file_registry.iter().chain(fm.files.iter()) {
        if let Some((path, line)) = parse_file_ref(entry) {
            let is_bare = !path.to_string_lossy().contains('/');
            refs.push(FileRef {
                path,
                line,
                source: FileRefSource::Frontmatter,
                is_bare_filename: is_bare,
            });
        }
    }

    // From inline backtick code — filtered more carefully
    for cap in find_backtick_paths(body) {
        if let Some((path, line)) = parse_file_ref(&cap) {
            let is_bare = !path.to_string_lossy().contains('/');
            refs.push(FileRef {
                path,
                line,
                source: FileRefSource::InlineCode,
                is_bare_filename: is_bare,
            });
        }
    }

    refs
}

/// Parse a file reference like "src/auth.rs:42" into (path, optional line).
fn parse_file_ref(s: &str) -> Option<(PathBuf, Option<u32>)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Check for path:line format
    if let Some((path_str, line_str)) = s.rsplit_once(':') {
        if let Ok(line) = line_str.parse::<u32>() {
            return Some((PathBuf::from(path_str), Some(line)));
        }
    }

    Some((PathBuf::from(s), None))
}

/// Find backtick-enclosed strings that look like file paths.
fn find_backtick_paths(body: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut chars = body.chars().peekable();
    let mut in_backtick = false;
    let mut current = String::new();

    while let Some(ch) = chars.next() {
        if ch == '`' && !in_backtick {
            in_backtick = true;
            current.clear();
        } else if ch == '`' && in_backtick {
            in_backtick = false;
            if looks_like_path(&current) {
                paths.push(current.clone());
            }
        } else if in_backtick {
            current.push(ch);
        }
    }

    paths
}

/// Known code file extensions.
const CODE_EXTENSIONS: &[&str] = &[
    ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go",
    ".java", ".c", ".cpp", ".cc", ".cxx", ".h", ".hpp",
    ".kt", ".swift", ".rb", ".cs", ".scala",
];

/// Heuristic: does this backtick content look like a file path?
fn looks_like_path(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.contains(' ') {
        return false;
    }

    // Reject URLs
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://") {
        return false;
    }

    // Reject API routes / endpoints (start with / but no file extension)
    if s.starts_with('/') && !CODE_EXTENSIONS.iter().any(|ext| s.ends_with(ext)) {
        return false;
    }

    // Reject things that look like config keys, env vars, or CLI flags
    if s.starts_with("--") || s.starts_with('$') || s.contains('=') {
        return false;
    }

    // Reject package/class notation (e.g. com.example.Foo) unless it also has a file extension
    if s.contains('.') && !s.contains('/') {
        let has_code_ext = CODE_EXTENSIONS.iter().any(|ext| s.ends_with(ext));
        if !has_code_ext {
            return false;
        }
    }

    // Accept: has a slash with a code extension, or is a bare filename with a code extension
    if s.contains('/') {
        // Has path separators — likely a real path
        return CODE_EXTENSIONS.iter().any(|ext| s.ends_with(ext))
            || s.ends_with(".json")
            || s.ends_with(".yaml")
            || s.ends_with(".yml")
            || s.ends_with(".toml")
            || s.ends_with(".xml")
            || s.ends_with(".properties");
    }

    // Bare filename with a code extension
    CODE_EXTENSIONS.iter().any(|ext| s.ends_with(ext))
}

/// Extract [[wikilinks]] from markdown body, skipping code blocks and inline code.
fn extract_wikilinks(body: &str) -> Vec<String> {
    // First, strip fenced code blocks and inline code to avoid false positives
    let stripped = strip_code_spans(body);

    let mut links = Vec::new();
    let mut rest = stripped.as_str();

    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        if let Some(end) = rest.find("]]") {
            let link = &rest[..end];
            // Handle [[link|alias]] format
            let name = link.split('|').next().unwrap_or(link).trim();
            // Reject things that look like variable interpolation or code
            if !name.is_empty() && !name.starts_with('$') && !name.contains('(') {
                links.push(name.to_string());
            }
            rest = &rest[end + 2..];
        } else {
            break;
        }
    }

    links
}

/// Replace fenced code blocks and inline backtick spans with empty strings.
fn strip_code_spans(body: &str) -> String {
    let mut result = String::with_capacity(body.len());
    let mut lines = body.lines().peekable();
    let mut in_fenced_block = false;

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fenced_block = !in_fenced_block;
            result.push('\n');
            continue;
        }
        if in_fenced_block {
            result.push('\n');
            continue;
        }
        // Strip inline backtick spans
        let mut chars = line.chars().peekable();
        let mut in_backtick = false;
        while let Some(ch) = chars.next() {
            if ch == '`' {
                in_backtick = !in_backtick;
            } else if !in_backtick {
                result.push(ch);
            }
        }
        result.push('\n');
    }

    result
}

/// Extract first meaningful paragraph as summary.
fn extract_summary(body: &str) -> String {
    let mut lines = Vec::new();

    for line in body.lines() {
        let trimmed = line.trim();
        // Skip headings, empty lines, and frontmatter artifacts
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("---") {
            if !lines.is_empty() {
                break; // End of first paragraph
            }
            continue;
        }
        lines.push(trimmed);
    }

    let summary = lines.join(" ");
    if summary.chars().count() > 300 {
        let truncated: String = summary.chars().take(297).collect();
        format!("{truncated}...")
    } else {
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter() {
        let content = "---\ntitle: Test\ntype: feature\n---\n# Body\nContent here.";
        let (fm, body) = split_frontmatter(content);
        assert!(fm.is_some());
        assert!(body.contains("Body"));
    }

    #[test]
    fn test_extract_wikilinks() {
        let body = "See [[Auth Flow]] and [[Session Manager|sessions]] for details.";
        let links = extract_wikilinks(body);
        assert_eq!(links, vec!["Auth Flow", "Session Manager"]);
    }

    #[test]
    fn test_extract_wikilinks_skips_code() {
        // Wikilinks inside inline code should be ignored
        let body = "See [[Real Link]] and `some code with [[fake link]]` here.";
        let links = extract_wikilinks(body);
        assert_eq!(links, vec!["Real Link"]);
    }

    #[test]
    fn test_extract_wikilinks_skips_fenced_blocks() {
        let body = "See [[Real Link]].\n\n```java\nmap.get([[key]]);\n```\n\nAnd [[Another Link]].";
        let links = extract_wikilinks(body);
        assert_eq!(links, vec!["Real Link", "Another Link"]);
    }

    #[test]
    fn test_extract_wikilinks_skips_variables() {
        // $variable syntax should not be treated as wikilinks
        let body = "Uses [[$varName]] and [[Real Note]].";
        let links = extract_wikilinks(body);
        assert_eq!(links, vec!["Real Note"]);
    }

    #[test]
    fn test_extract_summary_truncates_on_char_boundary() {
        // Regression: doctrack notes are full of em-dashes (3 bytes each).
        // The old code did `&summary[..297]`, which panicked when byte 297
        // landed inside a multibyte char. Build a body where an em-dash
        // straddles the old byte boundary and a long em-dash-heavy paragraph.
        let body = format!("{}\u{2014}{}", "a".repeat(296), "b".repeat(400));
        let summary = extract_summary(&body);
        assert!(summary.ends_with("..."));
        assert!(summary.chars().count() <= 300);

        let dashes = "\u{2014} ".repeat(400);
        let summary = extract_summary(&dashes);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn test_extract_summary_short_unchanged() {
        let summary = extract_summary("A short note \u{2014} with an em-dash.");
        assert_eq!(summary, "A short note \u{2014} with an em-dash.");
    }

    #[test]
    fn test_parse_file_ref_with_line() {
        let (path, line) = parse_file_ref("src/auth.rs:42").unwrap();
        assert_eq!(path, PathBuf::from("src/auth.rs"));
        assert_eq!(line, Some(42));
    }

    #[test]
    fn test_parse_file_ref_without_line() {
        let (path, line) = parse_file_ref("src/auth.rs").unwrap();
        assert_eq!(path, PathBuf::from("src/auth.rs"));
        assert_eq!(line, None);
    }

    #[test]
    fn test_looks_like_path() {
        // Valid paths
        assert!(looks_like_path("src/main.rs"));
        assert!(looks_like_path("auth.py"));
        assert!(looks_like_path("CertificateInfo.java"));
        assert!(looks_like_path("src/config/application.yaml"));

        // Rejected: URLs
        assert!(!looks_like_path("http://localhost:8761/eureka/"));
        assert!(!looks_like_path("https://example.com"));

        // Rejected: API routes
        assert!(!looks_like_path("/actuator/health"));
        assert!(!looks_like_path("/story"));
        assert!(!looks_like_path("/api/v1/users"));

        // Rejected: package notation without extension
        assert!(!looks_like_path("com.example.Foo"));

        // Rejected: misc
        assert!(!looks_like_path("some text"));
        assert!(!looks_like_path(""));
        assert!(!looks_like_path("--verbose"));
        assert!(!looks_like_path("$HOME"));
    }
}
