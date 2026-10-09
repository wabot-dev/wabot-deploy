//! Release notes, as something the console can render.
//!
//! GitHub sends them as Markdown. Rendering Markdown means either a
//! dependency that parses all of it and emits HTML — which then has to
//! be sanitised, because this text comes off the internet — or a small
//! parser for the part release notes actually use.
//!
//! This is the second. It reads headings, bullets, paragraphs, fenced
//! code, inline code and links, and treats everything else as text.
//! Nothing here produces HTML: it produces *structure*, and the
//! console renders it with `rsx!`, which escapes every value. A
//! release note containing `<script>` is a paragraph that says
//! `<script>`.

/// A run of text inside a paragraph, bullet or heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Link { text: String, url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(Vec<Inline>),
    Paragraph(Vec<Inline>),
    List(Vec<Vec<Inline>>),
    Code(String),
}

/// Parse notes into blocks. Never fails: the worst case is a document
/// of paragraphs, which is a readable way to be wrong.
pub fn parse(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut bullets: Vec<Vec<Inline>> = Vec::new();
    let mut code: Option<Vec<String>> = None;

    // Both accumulate until something else starts, so both have to be
    // closed in each branch below — hence the closures rather than a
    // repeated four lines.
    macro_rules! flush_paragraph {
        () => {
            if !paragraph.is_empty() {
                blocks.push(Block::Paragraph(inline(&paragraph.join(" "))));
                paragraph.clear();
            }
        };
    }
    macro_rules! flush_bullets {
        () => {
            if !bullets.is_empty() {
                blocks.push(Block::List(std::mem::take(&mut bullets)));
            }
        };
    }

    for line in markdown.lines() {
        let trimmed = line.trim();

        // A fence swallows everything until the next one, including
        // what would otherwise look like a heading or a bullet. That
        // is the whole point of a code block.
        if let Some(lines) = &mut code {
            if trimmed.starts_with("```") {
                blocks.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line.to_string());
            }
            continue;
        }
        if trimmed.starts_with("```") {
            flush_paragraph!();
            flush_bullets!();
            code = Some(Vec::new());
            continue;
        }

        if trimmed.is_empty() {
            flush_paragraph!();
            flush_bullets!();
            continue;
        }

        if let Some(text) = trimmed.strip_prefix('#') {
            flush_paragraph!();
            flush_bullets!();
            // One level, whatever depth was written: these render
            // inside a card that already has a heading above them, and
            // six sizes of heading in a release note is noise.
            blocks.push(Block::Heading(inline(text.trim_start_matches('#').trim())));
            continue;
        }

        if let Some(item) = bullet(trimmed) {
            flush_paragraph!();
            bullets.push(inline(item));
            continue;
        }

        flush_bullets!();
        paragraph.push(trimmed.to_string());
    }

    if let Some(lines) = code {
        blocks.push(Block::Code(lines.join("\n")));
    }
    flush_paragraph!();
    flush_bullets!();
    blocks
}

/// The text of a bullet, if the line is one.
fn bullet(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    None
}

/// Split a line into text, `code` and `[links](urls)`.
///
/// Emphasis is deliberately left alone: `*` appears in release notes
/// as a bullet marker and inside command lines, and stripping it turns
/// `rm -rf *` into `rm -rf`.
fn inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`') {
            if let Some(end) = after.find('`') {
                push_text(&mut out, &mut plain);
                out.push(Inline::Code(after[..end].to_string()));
                rest = &after[end + 1..];
                continue;
            }
        }

        if rest.starts_with('[') {
            if let Some(link) = link_at(rest) {
                push_text(&mut out, &mut plain);
                out.push(Inline::Link {
                    text: link.text,
                    url: link.url,
                });
                rest = &rest[link.length..];
                continue;
            }
        }

        // Byte by byte would split a multi-byte character; the first
        // char is the unit, and everything before it has already been
        // handled.
        let mut chars = rest.chars();
        if let Some(first) = chars.next() {
            plain.push(first);
            rest = chars.as_str();
        }
    }

    push_text(&mut out, &mut plain);
    out
}

fn push_text(out: &mut Vec<Inline>, plain: &mut String) {
    if !plain.is_empty() {
        out.push(Inline::Text(std::mem::take(plain)));
    }
}

struct Link {
    text: String,
    url: String,
    length: usize,
}

/// `[text](url)` at the start of `rest`, if that is what is there.
///
/// Only `http` and `https` links become links. A release note is text
/// from the internet rendered in an administrator's browser, and
/// `javascript:` in an `href` is the oldest way to make that matter.
fn link_at(rest: &str) -> Option<Link> {
    let close = rest.find("](")?;
    let end = rest[close..].find(')')? + close;
    let text = &rest[1..close];
    let url = &rest[close + 2..end];

    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    Some(Link {
        text: text.to_string(),
        url: url.to_string(),
        length: end + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Inline {
        Inline::Text(value.to_string())
    }

    #[test]
    fn headings_bullets_and_paragraphs() {
        let blocks = parse("## What changed\n\n- one thing\n- another\n\nAnd a sentence.\n");

        assert_eq!(
            blocks,
            vec![
                Block::Heading(vec![text("What changed")]),
                Block::List(vec![vec![text("one thing")], vec![text("another")]]),
                Block::Paragraph(vec![text("And a sentence.")]),
            ]
        );
    }

    /// Lines of one paragraph are one paragraph: Markdown joins them,
    /// and rendering each as its own would double the spacing of every
    /// wrapped note.
    #[test]
    fn a_wrapped_paragraph_stays_one_paragraph() {
        let blocks = parse("first line\nsecond line");
        assert_eq!(
            blocks,
            vec![Block::Paragraph(vec![text("first line second line")])]
        );
    }

    #[test]
    fn code_fences_swallow_what_would_otherwise_be_markup() {
        let blocks = parse("```sh\n# not a heading\n- not a bullet\n```");
        assert_eq!(
            blocks,
            vec![Block::Code("# not a heading\n- not a bullet".into())]
        );
    }

    /// An unterminated fence still has to produce its content —
    /// dropping it would silently lose the end of a note.
    #[test]
    fn an_unclosed_fence_is_still_code() {
        assert_eq!(
            parse("```\nsomething"),
            vec![Block::Code("something".into())]
        );
    }

    #[test]
    fn inline_code_and_links_come_out_typed() {
        let blocks = parse("Run `install` — see [the docs](https://example.com/x).");
        assert_eq!(
            blocks,
            vec![Block::Paragraph(vec![
                text("Run "),
                Inline::Code("install".into()),
                text(" — see "),
                Inline::Link {
                    text: "the docs".into(),
                    url: "https://example.com/x".into()
                },
                text("."),
            ])]
        );
    }

    /// This text comes off the internet and is rendered in an
    /// administrator's browser. `rsx!` escapes the values; the scheme
    /// check is the other half.
    #[test]
    fn only_http_links_are_links() {
        let blocks = parse("[click](javascript:alert(1))");
        assert_eq!(
            blocks,
            vec![Block::Paragraph(vec![text("[click](javascript:alert(1))")])]
        );
    }

    #[test]
    fn a_stray_backtick_is_just_a_character() {
        assert_eq!(parse("a ` b"), vec![Block::Paragraph(vec![text("a ` b")])]);
    }

    /// Multi-byte characters must not be split — release notes are
    /// written in Spanish here.
    #[test]
    fn accents_survive() {
        assert_eq!(
            parse("configuración `así`"),
            vec![Block::Paragraph(vec![
                text("configuración "),
                Inline::Code("así".into())
            ])]
        );
    }

    #[test]
    fn nothing_in_is_nothing_out() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n  \n").is_empty());
    }
}
