//! Models write Markdown; Telegram shows formatting only from its own small
//! HTML dialect. This turns the first into the second.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// `markdown` as Telegram HTML.
pub(super) fn to_html(markdown: &str) -> String {
    let mut page = Page::default();
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => page.open(tag),
            Event::End(tag) => page.close(tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => page.text(&text),
            Event::Code(code) => {
                page.html.push_str("<code>");
                page.text(&code);
                page.html.push_str("</code>");
            }
            Event::SoftBreak | Event::HardBreak => page.html.push('\n'),
            Event::Rule => {
                page.html.push_str("———");
                page.end_block();
            }
            _ => {}
        }
    }
    page.html.trim_end().to_owned()
}

/// `text` shown exactly as written, whatever characters it holds.
pub(super) fn escape(text: &str) -> String {
    let mut page = Page::default();
    page.text(text);
    page.html
}

#[derive(Default)]
struct Page {
    html: String,
    /// One entry per list being written: the next number, for a numbered one.
    lists: Vec<Option<u64>>,
}

impl Page {
    fn open(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { .. } | Tag::Strong => self.html.push_str("<b>"),
            Tag::Emphasis => self.html.push_str("<i>"),
            Tag::Strikethrough => self.html.push_str("<s>"),
            Tag::BlockQuote(_) => self.html.push_str("<blockquote>"),
            Tag::CodeBlock(CodeBlockKind::Fenced(language)) if !language.is_empty() => {
                self.html.push_str("<pre><code class=\"language-");
                self.text(&language);
                self.html.push_str("\">");
            }
            Tag::CodeBlock(_) => self.html.push_str("<pre><code>"),
            Tag::List(first) => {
                if !self.lists.is_empty() {
                    self.end_line();
                }
                self.lists.push(first);
            }
            Tag::Item => {
                self.html.push_str(&"  ".repeat(self.lists.len().saturating_sub(1)));
                match self.lists.last_mut() {
                    Some(Some(number)) => {
                        self.html.push_str(&format!("{number}. "));
                        *number += 1;
                    }
                    _ => self.html.push_str("• "),
                }
            }
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.html.push_str("<a href=\"");
                self.text(&dest_url);
                self.html.push_str("\">");
            }
            _ => {}
        }
    }

    fn close(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.end_block(),
            TagEnd::Heading(_) => {
                self.html.push_str("</b>");
                self.end_block();
            }
            TagEnd::Strong => self.html.push_str("</b>"),
            TagEnd::Emphasis => self.html.push_str("</i>"),
            TagEnd::Strikethrough => self.html.push_str("</s>"),
            TagEnd::BlockQuote(_) => {
                self.html.truncate(self.html.trim_end().len());
                self.html.push_str("</blockquote>");
                self.end_block();
            }
            TagEnd::CodeBlock => {
                self.html.push_str("</code></pre>");
                self.end_block();
            }
            TagEnd::Item => self.end_line(),
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.end_block();
                }
            }
            TagEnd::Link | TagEnd::Image => self.html.push_str("</a>"),
            _ => {}
        }
    }

    /// Text shown exactly as written, whatever characters it holds.
    fn text(&mut self, text: &str) {
        for character in text.chars() {
            match character {
                '&' => self.html.push_str("&amp;"),
                '<' => self.html.push_str("&lt;"),
                '>' => self.html.push_str("&gt;"),
                '"' => self.html.push_str("&quot;"),
                other => self.html.push(other),
            }
        }
    }

    fn end_line(&mut self) {
        if !self.html.ends_with('\n') {
            self.html.push('\n');
        }
    }

    /// Leaves exactly one empty line before whatever comes next.
    fn end_block(&mut self) {
        self.html.truncate(self.html.trim_end_matches('\n').len());
        self.html.push_str("\n\n");
    }
}
