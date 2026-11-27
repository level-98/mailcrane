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

/// Stands for a `<br>`, `</p>` or `&#10;`, whose line break `html_to_text`
/// writes itself: only a line break right after this mark is the sender's.
/// One without it comes from a `<br>` or `</p>` inside content
/// html_to_text leaves out (a template, a style), and counts as a block
/// boundary.
const MARK_BREAK: char = '\u{2}';

/// Stands before a space or tab the sender wrote as a character reference
/// (`&#32;`, `&Tab;`): the next character is kept even right after a block
/// boundary, where collapsed source whitespace is dropped (#456 review).
const MARK_KEEP: char = '\u{4}';

/// Whether `c` is one of the marks.
fn is_mark(c: char) -> bool {
    matches!(c, MARK | MARK_BREAK | MARK_KEEP)
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

/// Where the tag whose `<` is at `lt` ends, read as `html_to_text` reads
/// it: `Ok(after)` if a `>` closes it, else `Err(next)`, the `<` that
/// starts the next tag or the end. After the `<`, spaces and `/` are
/// skipped; a comment ends at its `-->`. The name runs up to a space; after
/// it an attribute's `=` takes a value, and a quoted value is skipped
/// whole, so a `<` or `>` in it counts for nothing (#456 review).
fn tag_end(bytes: &[u8], lt: usize) -> Result<usize, usize> {
    let end = bytes.len();
    let mut pos = lt.saturating_add(1);
    while bytes
        .get(pos)
        .is_some_and(|b| *b == b'/' || is_tag_space(*b))
    {
        pos = pos.saturating_add(1);
    }
    if bytes
        .get(pos..)
        .is_some_and(|rest| rest.starts_with(b"!--"))
    {
        return comment_end(bytes, pos.saturating_add(1)).ok_or(end);
    }
    // The name (a `/` in it included), up to a space.
    loop {
        match bytes.get(pos) {
            None => return Err(end),
            Some(b'>') => return Ok(pos.saturating_add(1)),
            Some(b'<') => return Err(pos),
            Some(b) if is_tag_space(*b) => break,
            Some(_) => pos = pos.saturating_add(1),
        }
    }
    // Spaces and `/`; then the first attribute byte, whatever it is.
    while bytes
        .get(pos)
        .is_some_and(|b| *b == b'/' || is_tag_space(*b))
    {
        pos = pos.saturating_add(1);
    }
    match bytes.get(pos) {
        None => return Err(end),
        Some(b'>') => return Ok(pos.saturating_add(1)),
        Some(b'<') => return Err(pos),
        Some(_) => pos = pos.saturating_add(1),
    }
    // Attributes: only `<`, `>` and `=` count.
    loop {
        let at = bytes
            .get(pos..)
            .and_then(|rest| rest.iter().position(|b| matches!(b, b'<' | b'>' | b'=')));
        let Some(at) = at else {
            return Err(end);
        };
        pos = pos.saturating_add(at);
        match bytes.get(pos) {
            Some(b'>') => return Ok(pos.saturating_add(1)),
            Some(b'=') => pos = attribute_value_end(bytes, pos.saturating_add(1)),
            _ => return Err(pos),
        }
    }
}

/// Where an attribute value starting at `pos` ends, as `html_to_text` has
/// it: after leading whitespace, a quoted value runs to its closing quote
/// (or the end, if there is none); an unquoted one ends at once.
fn attribute_value_end(bytes: &[u8], pos: usize) -> usize {
    let value = bytes.get(pos..).unwrap_or_default().trim_ascii_start();
    let after = match value.split_first() {
        Some((&quote @ (b'"' | b'\''), quoted)) => quoted
            .iter()
            .position(|b| *b == quote)
            .and_then(|close| quoted.get(close.saturating_add(1)..))
            .unwrap_or_default(),
        _ => value,
    };
    bytes.len().saturating_sub(after.len())
}

/// Where a comment whose `--` starts at `start` ends (after its `>`), as
/// `html_to_text` has it: at the first `>` after `--`, or after `--!` when
/// the body is more than the opening `--`. `None` if it never ends.
fn comment_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut pos = start;
    loop {
        let close = pos.saturating_add(bytes.get(pos..)?.iter().position(|b| *b == b'>')?);
        let comment = bytes.get(start..close)?;
        if comment.ends_with(b"--")
            || comment
                .strip_prefix(b"--")
                .is_some_and(|body| body.ends_with(b"--!"))
        {
            return Some(close.saturating_add(1));
        }
        pos = close.saturating_add(1);
    }
}

/// Calls `f` with the position and marker of each tag that gets one, in
/// order: every block-level start or end tag, `<br>` and `</p>` that
/// `html_to_text` closes. One forward pass over the tags as it reads them
/// ([`tag_end`]), so linear in the input.
fn each_marker(bytes: &[u8], mut f: impl FnMut(usize, &'static str)) {
    let mut pos = 0;
    while let Some(at) = bytes
        .get(pos..)
        .and_then(|rest| rest.iter().position(|b| *b == b'<'))
    {
        let lt = pos.saturating_add(at);
        pos = match tag_end(bytes, lt) {
            Ok(after) => {
                if let Some(marker) = marker_at(bytes, lt) {
                    f(lt, marker);
                }
                after
            }
            Err(next) => next,
        };
        pos = pos.max(lt.saturating_add(1));
    }
}

/// The marker for the tag starting at `lt`, if it is one that gets one.
fn marker_at(bytes: &[u8], lt: usize) -> Option<&'static str> {
    let mut name_start = lt.saturating_add(1);
    while bytes
        .get(name_start)
        .is_some_and(|b| *b == b'/' || is_tag_space(*b))
    {
        name_start = name_start.saturating_add(1);
    }
    // The name as html_to_text has it: up to `<`, `>` or a space, a `/`
    // inside included (`<br/x>` is `br/x`, no break), trailing ones not
    // (`<br/>` is `br`). Within the tag, so linear over `each_marker`.
    let len = bytes
        .get(name_start..)
        .unwrap_or_default()
        .iter()
        .take_while(|b| !matches!(b, b'<' | b'>') && !is_tag_space(**b))
        .count();
    let name = bytes
        .get(name_start..name_start.saturating_add(len))
        .unwrap_or_default();
    let name = name
        .iter()
        .rposition(|b| *b != b'/')
        .and_then(|last| name.get(..=last))
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
/// decodes to a line break (`&#10;`, `&NewLine;`) gets [`MARK_BREAK`] in
/// front, so its line break is the sender's, like a `<br>`; one that
/// decodes to a space or tab gets [`MARK_KEEP`] (#456 review). References
/// are found and decoded as `html_to_text` does it, with mail-parser's own
/// decoder. Adds at most one byte per four (`&#9;`).
fn without_marks(html: &str) -> String {
    let bytes = html.as_bytes();
    let is_special = |b: &u8| *b == b'&' || is_mark(char::from(*b));
    let mut out = String::with_capacity(html.len().saturating_add(html.len() / 4));
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
            (Some('\n'), None) => out.push(MARK_BREAK),
            (Some(' ' | '\t'), None) => out.push(MARK_KEEP),
            _ => {}
        }
    }
    out.push_str(html.get(copied..).unwrap_or_default());
    out
}

/// `html` with [`MARKER`] in front of every block-level start or end tag,
/// and [`MARKER_BREAK`] in front of each `<br>` and `</p>`. Nothing else
/// is changed: what is text is left to `html_to_text` alone, so no content
/// can be lost here. Only tags `html_to_text` closes get one
/// ([`each_marker`]); names are read as it reads them: after `<`, spaces
/// and `/` are skipped (a `/` makes it an end tag), and the name runs up to
/// `<`, `>`, `/` or a space. Marks already in the input are replaced first.
/// The result is at most twice the input (three bytes added per closed tag,
/// which is at least three, `<p>`; one per `&#9;`), allocated exactly;
/// checked positions, no panics on any input.
fn mark_html_blocks(html: &str) -> String {
    let html = without_marks(html);
    let bytes = html.as_bytes();
    let mut extra = 0usize;
    each_marker(bytes, |_, marker| {
        extra = extra.saturating_add(marker.len())
    });
    let mut out = String::with_capacity(html.len().saturating_add(extra));
    let mut copied = 0;
    each_marker(bytes, |lt, marker| {
        out.push_str(html.get(copied..lt).unwrap_or_default());
        out.push_str(marker);
        copied = lt;
    });
    out.push_str(html.get(copied..).unwrap_or_default());
    out
}

/// `html_to_text`'s output with each run of block marks made one line
/// break, where text came before it on the line; collapsed source spaces
/// after a block mark are dropped. A line break `html_to_text` wrote for a
/// sender's `<br>`, `</p>` or `&#10;` (the character right after
/// [`MARK_BREAK`]) is kept, after the break of a block mark before it, so
/// a blank line the sender made (`<div><br></div>`, `<br><br>`) stays. A
/// break mark followed by anything else changes nothing (`<br/x>`, which
/// html_to_text reads as a tag named `br/x`). The character after
/// [`MARK_KEEP`] is always kept. Any other line break counts as a block
/// mark. Without block marks, the result is `html_to_text`'s output
/// exactly (a property test checks it).
fn lines_from_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending = false;
    let mut after_break = false;
    let mut keep_next = false;
    let line_started = |out: &String| !out.is_empty() && !out.ends_with('\n');
    for c in text.chars() {
        let after_break_mark = std::mem::take(&mut after_break);
        let keep = std::mem::take(&mut keep_next);
        match c {
            MARK => pending = true,
            MARK_BREAK => after_break = true,
            MARK_KEEP => keep_next = true,
            '\n' if after_break_mark => {
                if pending && line_started(&out) {
                    out.push('\n');
                }
                out.push('\n');
                pending = false;
            }
            '\n' if !keep => pending = true,
            ' ' | '\t' if pending && !keep => {}
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
            // Cut off after a space or `/` (#456 review, round 5).
            "A<div <b> B",
            "A<div/<b> B",
            "A</div <i> B",
            "A<p <b> B",
            "A<p class=x <b> B",
            "A<td <span>B",
            "A<div title=\"x\" <b> B",
            // Or never closed: a quote that never ends, or a comment.
            "A<div title=\"> B",
            "A<!-- <div> B",
        ] {
            let plain = mail_parser::html_to_text(html);
            assert_eq!(html_to_lines(html), tidy_lines(&plain), "{html:?}");
        }
        assert_eq!(html_to_lines("A<br<b> B"), "A B");
        assert_eq!(html_to_lines("A<td <span>B"), "AB");
        // A `>` or `<` in a quoted value doesn't end or cut the tag.
        assert_eq!(html_to_lines("A<div title=\"<b>\">B</div>"), "A\nB");
        assert_eq!(html_to_lines("A<div title='1 > 0' >B"), "A\nB");
    }

    /// #456 review, round 5: whitespace the sender wrote as a reference
    /// right after a block boundary is kept; only collapsed source
    /// whitespace there is dropped.
    #[test]
    fn reference_whitespace_after_a_block_is_kept() {
        assert_eq!(html_to_lines("<li>&#32;&#32;x"), "  x");
        assert_eq!(html_to_lines("<div>A</div><div>&Tab;x</div>"), "A\n\tx");
        assert_eq!(html_to_lines("<div>A</div>&#32;&#32;B"), "A\n  B");
        assert_eq!(html_to_lines("<pre>&#32;&#32;code</pre>"), "  code");
        assert_eq!(html_to_lines("<div>A</div> \n <div> B</div>"), "A\nB");
    }

    /// Pieces of HTML for the differential property tests: text, spaces,
    /// inline tags, references, cut-off and quoted tags, comments, mark
    /// characters. No block tags, and no hidden content (whose `<br>`
    /// html_to_text writes as a bare line break).
    const INLINE_PIECES: &[&str] = &[
        "A",
        "bc",
        " ",
        "  ",
        "\t",
        "\n",
        "\r\n",
        "<b>",
        "</b>",
        "<i >",
        "< /i>",
        "<span class=\"x\">",
        "<a title=\"1 > 0\">",
        "<a alt='<b>'>",
        "<a x=y>",
        "<a =x>",
        "<br>",
        "<BR/>",
        "</br>",
        "<br/x>",
        "</p>",
        "</P >",
        "&#32;",
        "&#x20;",
        "&Tab;",
        "&#9;",
        "&#10;",
        "&NewLine;",
        "&amp;",
        "&nbsp;",
        "&#57344;",
        "&#1;",
        "&#+2;",
        "&#4;",
        "&#3;",
        "\u{1}",
        "\u{2}",
        "\u{4}",
        "&",
        ";",
        "#",
        "<",
        "<b",
        "<br",
        "</p",
        "<br ",
        "<br/",
        "</p ",
        "=",
        "\"",
        "'",
        ">",
        "/",
        "<!--",
        "-->",
        "<!-- x -->",
        "--!>",
        "<!-->",
        "\u{c}",
        "\u{a0}",
    ];

    /// The same with block tags and hidden content added.
    const BLOCK_PIECES: &[&str] = &[
        "<div>",
        "<p/>",
        "</div>",
        "<td>",
        "</td>",
        "<li>",
        "<DIV class=\"a>b\">",
        "< /div>",
        "<p>",
        "<p ",
        "<div ",
        "<div/",
        "<td ",
        "<pre>",
        "</pre>",
        "<style>",
        "</style>",
        "<script>",
        "</script>",
        "<template>",
        "</template>",
        "<head>",
        "</head>",
    ];

    fn html_from(pieces: &'static [&'static str]) -> impl Strategy<Value = String> {
        proptest::collection::vec(proptest::sample::select(pieces), 0..24).prop_map(|p| p.concat())
    }

    /// html_to_text's output as html_to_lines gives it: mark characters
    /// replaced, lines tidied.
    fn plain(html: &str) -> String {
        let text = mail_parser::html_to_text(html);
        tidy_lines(&text.replace(['\u{1}', '\u{2}', '\u{4}'], NOT_A_MARK))
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4096))]

        /// #456 (lead's decision): without block tags, html_to_lines is
        /// html_to_text exactly. The markers are the only difference.
        #[test]
        fn without_blocks_it_is_html_to_text(html in html_from(INLINE_PIECES)) {
            prop_assert_eq!(html_to_lines(&html), plain(&html), "{:?}", html);
        }

        /// With block tags, only whitespace differs: no character is
        /// lost, added or moved.
        #[test]
        fn blocks_change_only_whitespace(
            html in (html_from(INLINE_PIECES), html_from(BLOCK_PIECES), html_from(INLINE_PIECES))
                .prop_map(|(a, b, c)| {
                    // Interleave so block pieces land between inline ones.
                    let mut s = a;
                    s.push_str(&b);
                    s.push_str(&c);
                    s
                })
        ) {
            let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            prop_assert_eq!(squeeze(&html_to_lines(&html)), squeeze(&plain(&html)), "{:?}", html);
        }
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
        // Exactly as html_to_text has it, the space after included.
        assert_eq!(html_to_lines("A&#xA;B &NewLine; C"), "A\nB\n C");
        // Indentation the sender wrote is kept (#456 review, round 5).
        assert_eq!(html_to_lines("a<br>&#32;&#32;x"), "a\n  x");
        assert_eq!(html_to_lines("a<br>&Tab;x"), "a\n\tx");
        assert_eq!(html_to_lines("a&#10;&#32;&#32;x"), "a\n  x");
        assert_eq!(html_to_lines("a&#10;&#x20;x"), "a\n x");
        assert_eq!(html_to_lines("A<br>&#10;&#10;&#32;&#32;B"), "A\n\n  B");
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
