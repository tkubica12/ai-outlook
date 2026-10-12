use eframe::egui::{self, Align, FontSelection, RichText};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default)]
pub struct Document {
    blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
enum Block {
    Text {
        spans: Vec<Span>,
        heading: Option<u8>,
        indent: usize,
        prefix: String,
        quoted: bool,
    },
    Code(String),
    Table(Vec<Vec<Vec<Span>>>),
    Rule,
}

#[derive(Clone, Debug, Default)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Default)]
struct Builder {
    document: Document,
    spans: Vec<Span>,
    heading: Option<u8>,
    bold: usize,
    italic: usize,
    strike: usize,
    quote: usize,
    code: bool,
    link: Option<String>,
    lists: Vec<Option<u64>>,
    prefix: String,
    table: Option<Vec<Vec<Vec<Span>>>>,
    row: Vec<Vec<Span>>,
}

impl Builder {
    fn text(&mut self, text: &str, code: bool) {
        self.spans.push(Span {
            text: text.into(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            code,
            link: self.link.clone(),
        });
    }

    fn flush(&mut self) {
        if self.spans.is_empty() || self.table.is_some() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        self.document.blocks.push(if self.code {
            Block::Code(spans.into_iter().map(|span| span.text).collect())
        } else {
            Block::Text {
                spans,
                heading: self.heading,
                indent: self.lists.len().saturating_sub(1).min(8),
                prefix: std::mem::take(&mut self.prefix),
                quoted: self.quote > 0,
            }
        });
    }
}

impl Document {
    /// Parse on the AI executor, not while painting an answer.
    pub fn parse(markdown: &str) -> Self {
        let mut builder = Builder::default();
        for event in Parser::new_ext(
            markdown,
            Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
        ) {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        builder.flush();
                        builder.heading = Some(level as u8);
                    }
                    Tag::CodeBlock(_) => {
                        builder.flush();
                        builder.code = true;
                    }
                    Tag::BlockQuote(_) => {
                        builder.flush();
                        builder.quote += 1;
                    }
                    Tag::List(start) => {
                        builder.flush();
                        builder.lists.push(start);
                    }
                    Tag::Item => {
                        builder.flush();
                        builder.prefix = match builder.lists.last_mut() {
                            Some(Some(number)) => {
                                let prefix = format!("{number}.");
                                *number = number.saturating_add(1);
                                prefix
                            }
                            _ => "-".into(),
                        };
                    }
                    Tag::Strong => builder.bold += 1,
                    Tag::Emphasis => builder.italic += 1,
                    Tag::Strikethrough => builder.strike += 1,
                    Tag::Link { dest_url, .. } => {
                        builder.link =
                            crate::calendar::safe_url(&dest_url).then(|| dest_url.into_string());
                    }
                    Tag::Table(_) => {
                        builder.flush();
                        builder.table = Some(Vec::new());
                    }
                    _ => {}
                },
                Event::End(tag) => match tag {
                    TagEnd::Paragraph | TagEnd::Item => builder.flush(),
                    TagEnd::Heading(_) => {
                        builder.flush();
                        builder.heading = None;
                    }
                    TagEnd::CodeBlock => {
                        builder.flush();
                        builder.code = false;
                    }
                    TagEnd::BlockQuote(_) => {
                        builder.flush();
                        builder.quote -= 1;
                    }
                    TagEnd::List(_) => {
                        builder.flush();
                        builder.lists.pop();
                    }
                    TagEnd::Strong => builder.bold -= 1,
                    TagEnd::Emphasis => builder.italic -= 1,
                    TagEnd::Strikethrough => builder.strike -= 1,
                    TagEnd::Link => builder.link = None,
                    TagEnd::TableCell => {
                        builder.row.push(std::mem::take(&mut builder.spans));
                    }
                    TagEnd::TableHead | TagEnd::TableRow => {
                        if let Some(table) = &mut builder.table {
                            table.push(std::mem::take(&mut builder.row));
                        }
                    }
                    TagEnd::Table => {
                        if let Some(table) = builder.table.take() {
                            builder.document.blocks.push(Block::Table(table));
                        }
                    }
                    _ => {}
                },
                Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                    builder.text(&text, builder.code);
                }
                Event::Code(text) => builder.text(&text, true),
                Event::SoftBreak => builder.text(" ", builder.code),
                Event::HardBreak => builder.text("\n", builder.code),
                Event::Rule => {
                    builder.flush();
                    builder.document.blocks.push(Block::Rule);
                }
                Event::TaskListMarker(checked) => {
                    builder.text(if checked { "[x] " } else { "[ ] " }, false);
                }
                Event::FootnoteReference(text) => builder.text(&format!("[{text}]"), false),
                _ => {}
            }
        }
        builder.flush();
        builder.document
    }

    pub fn show(&self, ui: &mut egui::Ui) {
        for (index, block) in self.blocks.iter().enumerate() {
            ui.push_id(index, |ui| {
                match block {
                    Block::Text {
                        spans,
                        heading,
                        indent,
                        prefix,
                        quoted,
                    } => {
                        ui.horizontal_top(|ui| {
                            ui.add_space(*indent as f32 * 16.0);
                            if !prefix.is_empty() {
                                ui.label(prefix);
                            }
                            if *quoted {
                                ui.weak(">");
                            }
                            show_spans(ui, spans, *heading, false);
                        });
                    }
                    Block::Code(text) => {
                        egui::Frame::new()
                            .fill(ui.visuals().faint_bg_color)
                            .inner_margin(10.0)
                            .corner_radius(6.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(text).monospace());
                            });
                    }
                    Block::Table(rows) => {
                        let columns = rows.first().map_or(1, Vec::len).max(1);
                        let width = (ui.available_width() / columns as f32 - 12.0).max(40.0);
                        egui::ScrollArea::horizontal()
                            .id_salt("table-scroll")
                            .show(ui, |ui| {
                                egui::Grid::new("table")
                                    .striped(true)
                                    .spacing([12.0, 8.0])
                                    .show(ui, |ui| {
                                        for (row_index, row) in rows.iter().enumerate() {
                                            for cell in row {
                                                ui.vertical(|ui| {
                                                    ui.set_width(width);
                                                    show_spans(ui, cell, None, row_index == 0);
                                                });
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                    Block::Rule => {
                        ui.separator();
                    }
                }
                ui.add_space(6.0);
            });
        }
    }
}

fn show_spans(ui: &mut egui::Ui, spans: &[Span], heading: Option<u8>, header: bool) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut links = Vec::new();
    let mut offset = 0;
    for span in spans {
        let mut text = RichText::new(&span.text);
        if span.bold || header || heading.is_some() {
            text = text.strong();
        }
        if span.italic {
            text = text.italics();
        }
        if span.strike {
            text = text.strikethrough();
        }
        if span.code {
            text = text.code();
        }
        if let Some(level) = heading {
            text = text.size(26.0 - f32::from(level.min(6)) * 2.0);
        }
        let end = offset + span.text.chars().count();
        if let Some(url) = &span.link {
            text = text.color(ui.visuals().hyperlink_color).underline();
            links.push((offset..end, url));
        }
        text.append_to(&mut job, ui.style(), FontSelection::Default, Align::Center);
        offset = end;
    }
    let galley = ui.painter().layout_job(job);
    let response = ui.add(egui::Label::new(galley.clone()).sense(egui::Sense::click()));
    if let Some(position) = response.hover_pos() {
        let cursor = galley.cursor_from_pos(position - response.rect.min);
        if let Some((_, url)) = links
            .iter()
            .find(|(range, _)| range.contains(&cursor.index.0))
        {
            response
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(*url);
            if response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab((*url).clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commonmark_styles_lists_code_and_tables_without_showing_markup() {
        let document = Document::parse(
            "# Heading\n\n**Bold** and *italic*, `code`, ~~old~~.\n\n3. First\n4. Second\n\n> Quote\n\n```rust\nlet x = 1;\n```\n\n| Name | Value |\n|---|---|\n| One | **Two** |",
        );
        assert!(
            matches!(&document.blocks[0], Block::Text { heading: Some(1), spans, .. } if spans[0].text == "Heading")
        );
        assert!(
            matches!(&document.blocks[1], Block::Text { spans, .. } if spans.iter().any(|span| span.bold && span.text == "Bold") && spans.iter().any(|span| span.italic && span.text == "italic") && spans.iter().any(|span| span.code && span.text == "code") && spans.iter().any(|span| span.strike && span.text == "old"))
        );
        assert!(matches!(&document.blocks[2], Block::Text { prefix, .. } if prefix == "3."));
        assert!(matches!(&document.blocks[3], Block::Text { prefix, .. } if prefix == "4."));
        assert!(matches!(
            &document.blocks[4],
            Block::Text { quoted: true, .. }
        ));
        assert!(matches!(&document.blocks[5], Block::Code(code) if code == "let x = 1;\n"));
        assert!(
            matches!(&document.blocks[6], Block::Table(rows) if rows.len() == 2 && rows[1][1][0].bold)
        );
    }

    #[test]
    fn links_are_http_only_and_images_never_load_remote_content() {
        let document = Document::parse(
            "[Safe](https://example.com/a) [Unsafe](javascript:alert) [Local](file:///secret) [Credentials](https://name:pass@example.com) ![Alt](https://example.com/image.png)\n\n\\*\\*literal\\*\\*",
        );
        let Block::Text { spans, .. } = &document.blocks[0] else {
            panic!("Expected paragraph")
        };
        assert_eq!(spans.iter().filter(|span| span.link.is_some()).count(), 1);
        assert_eq!(
            spans
                .iter()
                .find(|span| span.text == "Safe")
                .unwrap()
                .link
                .as_deref(),
            Some("https://example.com/a")
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "Alt" && span.link.is_none())
        );
        let Block::Text { spans, .. } = &document.blocks[1] else {
            panic!("Expected literal paragraph")
        };
        assert_eq!(
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "**literal**"
        );
    }
}
