//! Architecture docs as the `architecture-docs` skill keeps them: each
//! document's title from its front matter, and how current it is from the
//! skill's `docs-check --json` report.

use serde::Deserialize;

/// `docs-check.mjs --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Report {
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Finding {
    /// Repository-relative, `/`-separated.
    pub file: String,
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub changed: Vec<String>,
}

/// How a document stands, worst first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Standing {
    /// Its code changed since it was last checked.
    Stale,
    /// Never checked against its code, or its check points nowhere.
    Unverified,
    /// Broken links, malformed diagrams or front matter.
    Issues,
    Current,
}

impl Standing {
    pub fn label(self) -> &'static str {
        match self {
            Standing::Stale => "stale",
            Standing::Unverified => "unverified",
            Standing::Issues => "issues",
            Standing::Current => "current",
        }
    }
}

impl Report {
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    /// The findings about `file`.
    pub fn about<'a>(&'a self, file: &'a str) -> impl Iterator<Item = &'a Finding> + 'a {
        self.findings.iter().filter(move |f| f.file == file)
    }

    /// The worst thing said about `file`.
    pub fn standing(&self, file: &str) -> Standing {
        self.about(file)
            .map(|f| match f.kind.as_str() {
                "stale" => Standing::Stale,
                "unverified" | "source" => Standing::Unverified,
                _ => Standing::Issues,
            })
            .min()
            .unwrap_or(Standing::Current)
    }
}

/// `title:` from a document's front matter, if it has one.
pub fn title(text: &str) -> Option<&str> {
    let body = text.strip_prefix("---")?.trim_start_matches(['\r', '\n']);
    let end = body.find("\n---")?;
    body[..end].lines().find_map(|line| {
        let value = line.strip_prefix("title:")?.trim();
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        (!value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_stands_by_its_worst_finding() {
        let report = Report::parse(
            r#"{"head":"7d9fad6","documents":3,"findings":[
                {"file":"docs/a.md","kind":"link","message":"Broken link: x.md"},
                {"file":"docs/a.md","kind":"stale","message":"2 source file(s) changed","changed":["src/x.ts"]},
                {"file":"docs/b.md","kind":"source","message":"Source `src/gone.ts` does not exist."},
                {"file":"docs/c.md","kind":"mermaid","message":"A fenced block is not closed."}
            ]}"#,
        )
        .unwrap();
        assert_eq!(report.standing("docs/a.md"), Standing::Stale);
        assert_eq!(report.standing("docs/b.md"), Standing::Unverified);
        assert_eq!(report.standing("docs/c.md"), Standing::Issues);
        assert_eq!(report.standing("docs/d.md"), Standing::Current);
        assert_eq!(report.about("docs/a.md").count(), 2);
        assert!(Report::parse("docs-check: not json").is_none());
    }

    #[test]
    fn the_title_comes_from_front_matter_only() {
        assert_eq!(
            title("---\ntitle: \"Payments — states\"\nowner: x\n---\n# H\n"),
            Some("Payments — states")
        );
        assert_eq!(title("---\nowner: x\n---\ntitle: not this\n"), None);
        assert_eq!(title("# No front matter\n"), None);
    }
}
