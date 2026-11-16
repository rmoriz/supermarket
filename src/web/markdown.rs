//! Markdown rendering for cookbook READMEs and tool instructions.
//!
//! Ports `MarkdownHelper#render_markdown` and its `SupermarketRenderer`: the
//! Redcarpet option set (autolink, fenced code, tables, strikethrough, hard
//! wrap, XHTML tags) plus the post-processing that adds `target="_blank"` and
//! makes image sources protocol relative.

use pulldown_cmark::{html, Event, Options, Parser};

pub fn render(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);
    // Redcarpet runs with `hard_wrap`, so soft breaks become `<br />`.
    let parser = Parser::new_ext(text, options).map(|event| match event {
        Event::SoftBreak => Event::HardBreak,
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    postprocess(&out)
}

/// `SupermarketRenderer#postprocess`: blank anchors open a new tab, XHTML void
/// tags are self-closed and `http(s)` image sources become protocol relative.
fn postprocess(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 32);
    let mut rest = html;
    while let Some(index) = rest.find('<') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find('>') else {
            out.push_str(rest);
            return out;
        };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        if tag.starts_with("<a ") {
            let inner = &tag[3..tag.len() - 1];
            out.push_str("<a ");
            out.push_str(inner.trim_end());
            out.push_str(r#" target="_blank" rel="noopener">"#);
        } else if tag.starts_with("<img ") {
            out.push_str(&self_close_img(tag));
        } else if tag == "<br>" || tag == "<hr>" {
            out.push_str(&tag[..tag.len() - 1]);
            out.push_str(" />");
        } else {
            out.push_str(tag);
        }
    }
    out.push_str(rest);
    out
}

fn self_close_img(tag: &str) -> String {
    let mut tag = tag.to_owned();
    if let Some(start) = tag.find("src=\"http") {
        let after_scheme = start + "src=\"".len();
        if tag[after_scheme..].starts_with("https:") {
            tag.replace_range(after_scheme..after_scheme + 6, "");
        } else if tag[after_scheme..].starts_with("http:") {
            tag.replace_range(after_scheme..after_scheme + 5, "");
        }
    }
    if let Some(stripped) = tag.strip_suffix('>') {
        tag = format!("{} />", stripped.trim_end_matches('/').trim_end());
    }
    tag
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn headings_and_paragraphs() {
        assert_eq!(render("# redis"), "<h1>redis</h1>\n");
        assert_eq!(render("Installs redis."), "<p>Installs redis.</p>\n");
    }

    #[test]
    fn links_open_in_a_new_tab() {
        let html = render("[src](https://example.test/x)");
        assert!(html.contains(r#"target="_blank" rel="noopener""#), "{html}");
    }

    #[test]
    fn hard_wraps_become_xhtml_breaks() {
        assert!(render("a\nb").contains("<br />"));
    }

    #[test]
    fn fenced_code_blocks() {
        assert!(render("```\nx\n```").contains("<pre><code>x"));
    }
}
