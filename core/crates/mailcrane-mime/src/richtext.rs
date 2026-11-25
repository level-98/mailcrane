//! Rich text for compose (M2-09): a small document model with bold,
//! italic, links and lists, turned into the HTML part and the plain-text
//! part of a `multipart/alternative` message.
//!
//! Everything from the user is escaped; links are kept only for `https`,
//! `http` and `mailto`, anything else becomes plain text. The HTML uses only
//! tags the sanitiser keeps, so what is sent is what Mailcrane would show.

/// A run of text with the same formatting.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// A link target; only `https:`, `http:` and `mailto:` are kept.
    pub link: Option<String>,
}

/// A block of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Span>),
    BulletList(Vec<Vec<Span>>),
    NumberedList(Vec<Vec<Span>>),
}

/// A rich-text document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RichDoc {
    pub blocks: Vec<Block>,
}

/// Escapes text for HTML element content and attribute values.
fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\r' => {}
            '\n' => out.push_str("<br>"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
}

/// The link target if it's allowed: `https:`, `http:` or `mailto:`, with no
/// spaces or control characters.
pub fn safe_link(link: &str) -> Option<&str> {
    let l = link.trim();
    let lower = l.to_ascii_lowercase();
    let scheme_ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || (lower.starts_with("mailto:") && l.len() > 7);
    let clean = !l.chars().any(|c| c.is_whitespace() || c.is_control());
    (scheme_ok && clean && l.len() <= 2048).then_some(l)
}

fn spans_html(spans: &[Span], out: &mut String) {
    for s in spans {
        if s.text.is_empty() {
            continue;
        }
        let link = s.link.as_deref().and_then(safe_link);
        if let Some(href) = link {
            out.push_str("<a href=\"");
            escape(href, out);
            // The same `rel` the sanitiser adds, so it leaves our HTML as is.
            out.push_str("\" rel=\"noopener noreferrer\">");
        }
        if s.bold {
            out.push_str("<b>");
        }
        if s.italic {
            out.push_str("<i>");
        }
        escape(&s.text, out);
        if s.italic {
            out.push_str("</i>");
        }
        if s.bold {
            out.push_str("</b>");
        }
        if link.is_some() {
            out.push_str("</a>");
        }
    }
}

/// The HTML body (no `<html>`/`<body>` wrapper).
pub fn to_html(doc: &RichDoc) -> String {
    let mut out = String::new();
    for block in &doc.blocks {
        match block {
            Block::Paragraph(spans) => {
                out.push_str("<p>");
                spans_html(spans, &mut out);
                out.push_str("</p>");
            }
            Block::BulletList(items) | Block::NumberedList(items) => {
                let tag = if matches!(block, Block::BulletList(_)) {
                    "ul"
                } else {
                    "ol"
                };
                out.push_str(&format!("<{tag}>"));
                for item in items {
                    out.push_str("<li>");
                    spans_html(item, &mut out);
                    out.push_str("</li>");
                }
                out.push_str(&format!("</{tag}>"));
            }
        }
        out.push('\n');
    }
    out
}

fn spans_plain(spans: &[Span]) -> String {
    let mut out = String::new();
    for s in spans {
        out.push_str(&s.text);
        if let Some(href) = s.link.as_deref().and_then(safe_link) {
            let shown = href.strip_prefix("mailto:").unwrap_or(href);
            // No point repeating a link that is its own text.
            if s.text.trim() != shown && s.text.trim() != href {
                out.push_str(&format!(" <{href}>"));
            }
        }
    }
    out
}

/// The plain-text version: paragraphs separated by a blank line, lists as
/// `- ` and `1. `, links as `text <url>`.
pub fn to_plain(doc: &RichDoc) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in &doc.blocks {
        match block {
            Block::Paragraph(spans) => parts.push(spans_plain(spans)),
            Block::BulletList(items) => parts.push(
                items
                    .iter()
                    .map(|i| format!("- {}", spans_plain(i)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Block::NumberedList(items) => parts.push(
                items
                    .iter()
                    .enumerate()
                    .map(|(n, i)| format!("{}. {}", n + 1, spans_plain(i)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests;
