//! Short preview text for message lists.

/// How many characters the snippet holds.
pub const SNIPPET_CHARS: usize = 200;

/// The first [`SNIPPET_CHARS`] characters of `text`, with every run of
/// whitespace (and control characters) collapsed to one space.
pub fn snippet(text: &str) -> String {
    let mut out = String::new();
    let mut count: usize = 0;
    let mut pending_space = false;
    for c in text.chars() {
        if c.is_whitespace() || c.is_control() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            // Only with room for a character after it, so the snippet
            // never ends in a space.
            if count.saturating_add(1) >= SNIPPET_CHARS {
                break;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= SNIPPET_CHARS {
            break;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Block-level HTML tags whose start or end separates text (#391).
const BLOCK_TAGS: [&str; 40] = [
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "br",
    "li",
    "tr",
    "td",
    "th",
    "blockquote",
    "pre",
    "hr",
    "ul",
    "ol",
    "table",
    "dt",
    "dd",
    "dl",
    "section",
    "center",
    "header",
    "footer",
    "article",
    "aside",
    "nav",
    "main",
    "address",
    "caption",
    "figure",
    "figcaption",
    "details",
    "summary",
    "form",
    "fieldset",
    "legend",
    "option",
];

/// The longest name in [`BLOCK_TAGS`] (a test keeps it right).
const LONGEST_TAG: usize = 10;

/// `html` as plain text with a line break at every block boundary, so
/// text from adjacent blocks stays apart: on its own line in `text_plain`
/// (a draft's body, the reply quote) and one space apart in the snippet,
/// which collapses whitespace (#391, #446). `mail_parser::html_to_text`
/// alone decides what is text; block boundaries are marked before it runs
/// and turned into line breaks after (#456 review). A blank line the
/// sender made with `<br><br>` stays; runs of blank lines become one, and
/// the text starts and ends with neither.
pub(crate) fn html_to_lines(html: &str) -> String {
    let text = mail_parser::html_to_text(&mark_html_blocks(html));
    tidy_lines(&lines_from_marks(&text))
}

/// Stands for a line break between blocks while `html_to_text` runs: a
/// control character, one byte, meaningless in mail text (any in the input
/// are replaced first, see [`without_marks`]). Not a private-use character:
/// icon fonts put real glyphs there (#456 review).
const MARK: char = '\u{1}';

/// Stands for a `<br>` or `</p>`, whose line break `html_to_text` writes
/// itself: only a line break right after this mark is the sender's. One
/// without it comes from a `<br>` or `</p>` inside content html_to_text
/// leaves out (a template, a style), and counts as a block boundary.
const MARK_BREAK: char = '\u{2}';

/// Like [`MARK_BREAK`], for a line break written as a character reference
/// (`&#10;`). After one, `html_to_text` puts the collapsed space that
/// followed in the source; that one space is dropped.
const MARK_REFERENCE_BREAK: char = '\u{4}';

/// Whether `c` is one of the marks.
fn is_mark(c: char) -> bool {
    matches!(c, MARK | MARK_BREAK | MARK_REFERENCE_BREAK)
}

/// What goes before a block tag: `<>`, which `html_to_text` reads as an
/// empty tag (ending any tag it was still reading), then the mark as text.
/// So the mark reaches the output exactly when the block does: inside a
/// comment, a style or script, a template or a quoted attribute value it
/// is dropped with that content (#456 review). Three bytes, so marking at
/// most doubles the input (a `<p>` per three bytes).
const MARKER: &str = "<>\u{1}";

/// The same before a `<br>` or `</p>`.
const MARKER_BREAK: &str = "<>\u{2}";

/// Whether `b` is whitespace to `html_to_text`: space, tab, CR and LF (not
/// form feed, #456 review).
fn is_tag_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// The marker for the tag starting at `lt`, if it gets one.
fn marker_at(bytes: &[u8], lt: usize) -> Option<&'static str> {
    let mut name_start = lt.saturating_add(1);
    while bytes
        .get(name_start)
        .is_some_and(|b| *b == b'/' || is_tag_space(*b))
    {
        name_start = name_start.saturating_add(1);
    }
    let len = bytes
        .get(name_start..)
        .unwrap_or_default()
        .iter()
        .take(LONGEST_TAG.saturating_add(1))
        .take_while(|b| !matches!(b, b'<' | b'>' | b'/') && !is_tag_space(**b))
        .count();
    // A name cut off by `<` or the end never makes a tag (#456 review).
    let after = bytes.get(name_start.saturating_add(len)).copied();
    if !after.is_some_and(|b| matches!(b, b'>' | b'/') || is_tag_space(b)) {
        return None;
    }
    let name = bytes
        .get(name_start..name_start.saturating_add(len))
        .unwrap_or_default();
    let closing = bytes
        .get(lt.saturating_add(1)..name_start)
        .is_some_and(|between| between.contains(&b'/'));
    let is = |t: &str| t.as_bytes().eq_ignore_ascii_case(name);
    if is("br") || (closing && is("p")) {
        Some(MARKER_BREAK)
    } else if BLOCK_TAGS.iter().any(|t| is(t)) {
        Some(MARKER)
    } else {
        None
    }
}

/// What a mark in the input becomes: another control character, as
/// meaningless in text as the one it replaces. Replaced, not removed, so
/// removing one can never join what is around it into a new character
/// reference (`&&#1;#1;`).
const NOT_A_MARK: &str = "\u{3}";

/// Whether `b` ends a text token in `html_to_text`. A character reference
/// it decodes is `&`, then bytes that don't, then `;`.
fn ends_text_token(b: u8) -> bool {
    matches!(b, b'<' | b'&' | b';') || is_tag_space(b)
}

/// `html` with nothing in it that `html_to_text` would turn into a mark:
/// each mark character, raw or as a character reference it decodes
/// (`&#1;`, `&#x02;`, `&#+1;`), becomes [`NOT_A_MARK`]. A reference that
/// decodes to a line break (`&#10;`, `&NewLine;`) gets
/// [`MARK_REFERENCE_BREAK`] in front, so its line break stays the sender's
/// (#456 review). References are found and decoded as `html_to_text` does
/// it, with mail-parser's own decoder. Adds at most one byte per five.
fn without_marks(html: &str) -> String {
    let bytes = html.as_bytes();
    let is_special = |b: &u8| *b == b'&' || is_mark(char::from(*b));
    let mut out = String::with_capacity(html.len());
    let mut decoded = String::new();
    let mut copied = 0;
    let mut pos = 0;
    while let Some(at) = bytes
        .get(pos..)
        .and_then(|rest| rest.iter().position(is_special))
    {
        let start = pos.saturating_add(at);
        pos = start.saturating_add(1);
        out.push_str(html.get(copied..start).unwrap_or_default());
        copied = start;
        if bytes.get(start) != Some(&b'&') {
            out.push_str(NOT_A_MARK);
            copied = pos;
            continue;
        }
        let name_len = bytes
            .get(pos..)
            .unwrap_or_default()
            .iter()
            .take_while(|b| !ends_text_token(**b))
            .count();
        let semicolon = pos.saturating_add(name_len);
        if bytes.get(semicolon) != Some(&b';') {
            continue;
        }
        let end = semicolon.saturating_add(1);
        let token = bytes.get(start..end).unwrap_or_default();
        decoded.clear();
        mail_parser::add_html_token(&mut decoded, token, false);
        let mut chars = decoded.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if is_mark(c) => {
                out.push_str(NOT_A_MARK);
                copied = end;
                pos = end;
            }
            (Some('\n'), None) => out.push(MARK_REFERENCE_BREAK),
            _ => {}
        }
    }
    out.push_str(html.get(copied..).unwrap_or_default());
    out
}

/// `html` with [`MARKER`] in front of every block-level start or end tag,
/// and [`MARKER_BREAK`] in front of each `<br>` and `</p>`. Nothing else
/// is changed: what is text is left to `html_to_text` alone, so no content
/// can be lost here. Names are read as `html_to_text` reads them: after
/// `<`, spaces and `/` are skipped (a `/` makes it an end tag), and the
/// name runs up to `<`, `>`, `/` or a space; one cut off by `<` or the end
/// gets no marker. Marks already in the input are replaced first. The
/// result is at most twice the input (three bytes added per tag of at
/// least three, `<p>`; one per `&#10;`), allocated exactly; checked
/// positions, no panics on any input.
fn mark_html_blocks(html: &str) -> String {
    let html = without_marks(html);
    let bytes = html.as_bytes();
    let tags = || {
        bytes
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'<')
            .filter_map(|(lt, _)| marker_at(bytes, lt).map(|m| (lt, m)))
    };
    let extra = tags().count().saturating_mul(MARKER.len());
    let mut out = String::with_capacity(html.len().saturating_add(extra));
    let mut copied = 0;
    for (lt, marker) in tags() {
        out.push_str(html.get(copied..lt).unwrap_or_default());
        out.push_str(marker);
        copied = lt;
    }
    out.push_str(html.get(copied..).unwrap_or_default());
    out
}

/// `html_to_text`'s output with each run of block marks made one line
/// break, where text came before it on the line; spaces after a block mark
/// are dropped. A line break `html_to_text` wrote for a sender's `<br>`,
/// `</p>` or `&#10;` (the character right after [`MARK_BREAK`] or
/// [`MARK_REFERENCE_BREAK`]) is kept, after the break of a block mark
/// before it, so a blank line the sender made (`<div><br></div>`,
/// `<br><br>`) stays. A break mark followed by anything else was a tag
/// that never closed (`<br<b>`) and changes nothing. Any other line break
/// counts as a block mark.
fn lines_from_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending = false;
    let mut after_mark: Option<char> = None;
    let mut after_reference_break = false;
    let line_started = |out: &String| !out.is_empty() && !out.ends_with('\n');
    for c in text.chars() {
        let mark = after_mark.take();
        let skip_space = std::mem::take(&mut after_reference_break);
        match c {
            MARK => pending = true,
            MARK_BREAK | MARK_REFERENCE_BREAK => after_mark = Some(c),
            '\n' if mark.is_some() => {
                if pending && line_started(&out) {
                    out.push('\n');
                }
                out.push('\n');
                pending = false;
                after_reference_break = mark == Some(MARK_REFERENCE_BREAK);
            }
            '\n' => pending = true,
            ' ' | '\t' if pending => {}
            ' ' if skip_space => {}
            _ => {
                if pending && line_started(&out) {
                    out.push('\n');
                }
                pending = false;
                out.push(c);
            }
        }
    }
    out
}

/// `text` with trailing spaces cut from each line, no blank lines at the
/// start or end, and each run of blank lines made one.
fn tidy_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = false;
    for line in text.split('\n').map(str::trim_end) {
        if line.is_empty() {
            blank_run = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank_run {
                out.push('\n');
            }
        }
        blank_run = false;
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_tag_covers_every_block_tag() {
        assert!(BLOCK_TAGS.iter().all(|t| t.len() <= LONGEST_TAG));
        assert!(BLOCK_TAGS.iter().any(|t| t.len() == LONGEST_TAG));
    }

    /// #440 review: newsletter layouts use these blocks too.
    #[test]
    fn layout_blocks_are_lines() {
        assert_eq!(
            html_to_lines("<header>Acme</header><main>Hello</main><center>Sale</center>"),
            "Acme\nHello\nSale"
        );
    }

    #[test]
    fn block_tags_end_lines_and_inline_tags_dont() {
        assert_eq!(
            html_to_lines("<h1>A</h1><p>B<b>c</b></P><BR/>d<li>e"),
            "A\nBc\n\nd\ne"
        );
        // Names that only start like a block tag are inline.
        assert_eq!(html_to_lines("a<pa>b<br2>c<b>d"), "abcd");
        for odd in [
            "", "<", "</", "<<p", "a<", "<\u{e9}>", "<p", "</ p>", "<p<p>", "a>b<p>",
        ] {
            let _ = html_to_lines(odd);
        }
    }

    /// #446: adjacent blocks give one line each, not a blank line between;
    /// a blank line the sender made with `<br><br>` stays, once.
    #[test]
    fn adjacent_blocks_are_one_line_each_and_blank_lines_stay() {
        assert_eq!(
            html_to_lines("<div>Line 1</div><div>Line 2</div>"),
            "Line 1\nLine 2"
        );
        assert_eq!(
            html_to_lines("<div>Hi,</div><div><br></div><div>Thanks</div>"),
            "Hi,\n\nThanks"
        );
        assert_eq!(html_to_lines("A<br><br><br><br>B"), "A\n\nB");
        assert_eq!(html_to_lines("<p>A</p><p>B</p>"), "A\nB");
        // Text after a block's end still starts a new line.
        assert_eq!(html_to_lines("<div>A</div>B<div>C</div>"), "A\nB\nC");
        // Attributes aren't text.
        assert_eq!(
            html_to_lines(r#"<div class="x">A</div><div title="y">B</div>"#),
            "A\nB"
        );
    }

    /// #456 review: what `html_to_text` leaves out isn't text, so no blank
    /// line comes from a style block, a script, a template or a comment
    /// between blocks; nor from a `>` or `<` in a quoted attribute value.
    #[test]
    fn hidden_content_and_quoted_values_add_no_blank_line() {
        for between in [
            "<style>p { color: red }</style>",
            "<script>if (a > b) {}</script>",
            "<template><p>x</p></template>",
            "<!-- a > b -->",
            "<!-- <p>x</p> -->",
            r#"<div><a title="1 > 0"><img src=x></a></div>"#,
            r#"<img alt="x>y">"#,
            r#"<img alt='<p>'>"#,
        ] {
            assert_eq!(
                html_to_lines(&format!("<div>A</div>{between}<div>B</div>")),
                "A\nB",
                "{between}"
            );
        }
        // Text after a hidden element still counts.
        assert_eq!(html_to_lines("<style>x</style>A<div>B</div>"), "A\nB");
        // A head before the body is left out, then text counts again.
        assert_eq!(
            html_to_lines(
                "<html><head><title>T</title></head><body><p>A</p><p>B</p></body></html>"
            ),
            "A\nB"
        );
        for odd in ["<!--", "<!-- x", "<a title=\"x", "<style>", "<a title='>"] {
            let _ = html_to_lines(odd);
        }
    }

    /// #456 review: nothing after a stray `<` is lost, wherever it is;
    /// html_to_text alone decides what is text.
    #[test]
    fn a_stray_angle_bracket_loses_nothing() {
        assert_eq!(
            html_to_lines("<div>A</div><script>if (a < b) {}</script><div>B</div>"),
            "A\nB"
        );
        assert_eq!(
            html_to_lines("A<div>B</div><p>C < D</p><div>E</div>"),
            // html_to_text reads `< D</p` as a tag (as it always has); the
            // blocks around it keep their lines.
            "A\nB\nC\nE"
        );
        // html_to_text reads `<3 you` as a tag; the block after it still
        // starts a line.
        assert_eq!(html_to_lines("I <3 you<div>Next</div>"), "I\nNext");
        for name in ["head-x", "style:x", "script-x", "template-x"] {
            assert_eq!(
                html_to_lines(&format!("<div>A</div><{name}>m</{name}><div>B</div>")),
                "A\nm\nB",
                "{name}"
            );
        }
    }

    /// #456 review: a raw no-break space is text to html_to_text, so a block
    /// holding only one is a blank line, as `&nbsp;` is.
    #[test]
    fn a_block_of_a_no_break_space_is_a_blank_line() {
        assert_eq!(
            html_to_lines("<div>A</div><div>\u{a0}</div><div>B</div>"),
            "A\n\nB"
        );
        assert_eq!(
            html_to_lines("<div>A</div><div>&nbsp;</div><div>B</div>"),
            "A\n\nB"
        );
    }

    /// #456 review: tag names as html_to_text reads them.
    #[test]
    fn closing_tags_with_spaces_and_odd_names() {
        assert_eq!(html_to_lines("A</ p><div>B"), "A\nB");
        assert_eq!(html_to_lines("A< br>B"), "A\nB");
        // Not `</p>` to html_to_text: no line break, from it or from us.
        assert_eq!(html_to_lines("<div>A</p:x>B</div>"), "AB");
        assert_eq!(html_to_lines("<div>A</div><!--!>x--><div>B</div>"), "A\nB");
        // A form feed is no tag space to html_to_text, so it isn't here.
        assert_eq!(html_to_lines("A</\u{c}p> B<\u{c}br> C"), "A B C");
    }

    /// #456 review: a `<br>` or `</p>` cut off by `<` makes no tag, so
    /// no line break, and the words around it stay apart as html_to_text
    /// keeps them.
    #[test]
    fn a_tag_that_never_closes_changes_nothing() {
        for html in [
            "A<br<b> B",
            "A</p<i> B",
            "A<br <b> B",
            "A<br/<b> B",
            "A<div<b> B",
            "A<br",
        ] {
            let plain = mail_parser::html_to_text(html);
            assert_eq!(html_to_lines(html), tidy_lines(&plain), "{html:?}");
        }
        assert_eq!(html_to_lines("A<br<b> B"), "A B");
    }

    /// #456 review: a mark character in the input, raw or in any form of
    /// character reference html_to_text decodes, is no line break and
    /// swallows no spaces.
    #[test]
    fn marks_in_the_input_are_no_line_breaks() {
        let x = NOT_A_MARK;
        assert_eq!(html_to_lines("A\u{1}B\u{2}C"), format!("A{x}B{x}C"));
        assert_eq!(
            html_to_lines("A&#1;B&#x2;C&#X01; D&#0000000000002;  E&#+1;F"),
            format!("A{x}B{x}C{x} D{x} E{x}F")
        );
        // Removing one never makes a new reference.
        assert_eq!(html_to_lines("&&#1;#1;"), format!("&{x}#1;"));
        assert_eq!(html_to_lines("&#\u{1}1;"), format!("&#{x}1;"));
        // Inside a tag or a comment it goes with them.
        assert_eq!(html_to_lines("A<a title=\"&#1;\">B</a><!--&#2;-->C"), "ABC");
        // Other references are left to html_to_text.
        assert_eq!(html_to_lines("A&#65;B&amp;C&#x;D&#1"), "AAB&C&#x;D&#1");
    }

    /// #456 review: icon fonts (Material Icons) put glyphs in the
    /// private-use area; they are text like any other.
    #[test]
    fn private_use_characters_are_text() {
        for html in ["A&#xE000;B", "A&#57345;   B", "A\u{e000}B"] {
            let plain = mail_parser::html_to_text(html);
            assert_eq!(html_to_lines(html), tidy_lines(&plain), "{html:?}");
        }
        assert_eq!(html_to_lines("A&#57345;   B"), "A\u{e001} B");
        assert_eq!(html_to_lines("<div>A&#xe001;</div>B"), "A\u{e001}\nB");
    }

    /// #456 review: a line break written as a character reference is the
    /// sender's, like a `<br>`, and isn't merged with the breaks around it.
    #[test]
    fn a_line_break_reference_is_the_senders() {
        assert_eq!(html_to_lines("a<br>&#10;b"), "a\n\nb");
        assert_eq!(html_to_lines("A&#10;&#10;B"), "A\n\nB");
        // The source space html_to_text puts after it is dropped.
        assert_eq!(html_to_lines("A&#xA;B &NewLine; C"), "A\nB\nC");
        // Indentation the sender wrote is kept.
        assert_eq!(html_to_lines("a<br>&#32;&#32;x"), "a\n  x");
        assert_eq!(html_to_lines("a<br>&Tab;x"), "a\n\tx");
        assert_eq!(html_to_lines("a&#10;&#32;&#32;x"), "a\n x");
        assert_eq!(
            html_to_lines("<div>A</div><div>&#10;</div><div>B</div>"),
            "A\n\nB"
        );
        // Inside a tag or hidden content it adds nothing.
        assert_eq!(
            html_to_lines("<div title=&#10;>A</div><style>&#10;</style><div>B</div>"),
            "A\nB"
        );
    }

    /// #456 review: marking at most doubles the HTML, whatever it holds.
    #[test]
    fn marking_at_most_doubles_the_input() {
        for html in [
            "<p>".repeat(100_000),
            "<br>".repeat(100_000),
            "< /div>".repeat(50_000),
            "<p>&#10;".repeat(50_000),
            "<p>&#1;\u{1}".repeat(50_000),
            // Cut off by `<`: no tag, so no marker.
            "<p".repeat(100_000),
            "</p".repeat(100_000),
            "<br".repeat(100_000),
            "<p ".repeat(100_000),
            "<p/".repeat(100_000),
        ] {
            let marked = mark_html_blocks(&html);
            assert!(
                marked.len() <= html.len() * 2,
                "{} -> {}",
                html.len(),
                marked.len()
            );
            assert!(marked.capacity() <= html.len() * 2);
        }
    }

    #[test]
    fn tidy_lines_trims_and_keeps_one_blank_line() {
        assert_eq!(tidy_lines("\n\n a  \n\n\n\nb \n\n"), " a\n\nb");
        assert_eq!(tidy_lines(""), "");
        assert_eq!(tidy_lines("\n \n"), "");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(snippet("  a \r\n\t b  "), "a b");
    }

    #[test]
    fn never_ends_in_a_space_at_the_cap() {
        let text = format!("{} next", "x".repeat(SNIPPET_CHARS - 1));
        let s = snippet(&text);
        assert_eq!(s, "x".repeat(SNIPPET_CHARS - 1));
    }

    #[test]
    fn caps_length() {
        assert_eq!(snippet(&"x".repeat(500)).chars().count(), SNIPPET_CHARS);
        assert!(snippet(&"x ".repeat(500)).chars().count() <= SNIPPET_CHARS);
    }
}
