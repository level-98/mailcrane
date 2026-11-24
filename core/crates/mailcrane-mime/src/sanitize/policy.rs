//! The sanitiser's allow-lists, in one place.
//!
//! **Changing anything in this file needs a maintainer's approval** (see
//! CONTRIBUTING.md, "Decisions that only maintainers make"). Loosening an
//! allow-list can let message HTML run code or leak information. Anything
//! not listed here is removed.

/// Tags that survive. Everything else is removed; the contents of the tags in
/// [`CLEAN_CONTENT_TAGS`] are removed with them. Deliberately absent: `script`,
/// `style`, `iframe`, `object`, `embed`, `form`, `input`, `button`,
/// `textarea`, `select`, `meta`, `link`, `base`, `svg`, `math`, `video`,
/// `audio`, `source`, `track`, `picture`, `map`, `area`.
pub(super) const TAGS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "b",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "div",
    "dl",
    "dt",
    "em",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "li",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "tt",
    "u",
    "ul",
];

/// Tags removed together with everything inside them.
pub(super) const CLEAN_CONTENT_TAGS: &[&str] = &[
    "script",
    "style",
    "iframe",
    "object",
    "embed",
    "noscript",
    "template",
    "svg",
    "math",
    "video",
    "audio",
    "head",
    "title",
    "frameset",
    "noframes",
    "applet",
    "xmp",
    "plaintext",
];

/// Attributes allowed on every allowed tag. No `id`, `name` (DOM clobbering),
/// `class`, `srcset`, `target` or `on*` handlers.
pub(super) const GENERIC_ATTRIBUTES: &[&str] = &["style", "title", "dir", "lang"];

/// Attributes allowed on one tag.
pub(super) const TAG_ATTRIBUTES: &[(&str, &[&str])] = &[
    ("a", &["href"]),
    ("img", &["src", "alt", "width", "height"]),
    ("font", &["color", "face", "size"]),
    ("p", &["align"]),
    ("div", &["align"]),
    ("h1", &["align"]),
    ("h2", &["align"]),
    ("h3", &["align"]),
    ("h4", &["align"]),
    ("h5", &["align"]),
    ("h6", &["align"]),
    ("hr", &["align", "width", "size"]),
    (
        "table",
        &[
            "align",
            "bgcolor",
            "border",
            "cellpadding",
            "cellspacing",
            "width",
            "height",
        ],
    ),
    ("tr", &["align", "valign", "bgcolor"]),
    (
        "td",
        &[
            "align", "valign", "bgcolor", "width", "height", "colspan", "rowspan",
        ],
    ),
    (
        "th",
        &[
            "align", "valign", "bgcolor", "width", "height", "colspan", "rowspan",
        ],
    ),
    ("col", &["span", "width"]),
    ("colgroup", &["span", "width"]),
    ("ol", &["start", "type"]),
    ("li", &["value"]),
];

/// URL schemes allowed in `href`.
pub(super) const HREF_SCHEMES: &[&str] = &["https", "http", "mailto"];

/// Image types allowed in `data:` URLs in `img src`. SVG is not one of them.
pub(super) const DATA_IMAGE_TYPES: &[&str] = &["png", "jpeg", "gif", "webp"];

/// Schemes that can never be used for the internal `cid:` replacement URL.
pub(super) const FORBIDDEN_CID_SCHEMES: &[&str] = &[
    "javascript",
    "vbscript",
    "data",
    "file",
    "blob",
    "about",
    "http",
    "https",
    "mailto",
    "cid",
    "livescript",
    "mocha",
];

/// CSS properties kept in `style` attributes: colour, background colour,
/// font, text, spacing, border, size and alignment. Everything else (such as
/// `position`, `behavior`, `background-image`, `content`) is dropped.
pub(super) const CSS_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "line-height",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-transform",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "vertical-align",
    "direction",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-color",
    "border-style",
    "border-width",
    "border-radius",
    "border-collapse",
    "border-spacing",
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "list-style-type",
    "table-layout",
];

/// CSS functions allowed in values. `url(` and `expression(` are not.
pub(super) const CSS_FUNCTIONS: &[&str] = &["rgb", "rgba", "hsl", "hsla"];

/// A tracking pixel is an image this many pixels or smaller in either
/// direction.
pub(super) const TRACKER_MAX_PX: u32 = 2;
