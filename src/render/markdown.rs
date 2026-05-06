use anyhow::{anyhow, Result};
use crossterm::terminal;
use streamdown_ansi::utils::visible_length;
use streamdown_config::{ComputedStyle, Config as StreamdownConfig};
use streamdown_parser::{
    InlineElement, InlineParser, ListBullet, ParseEvent, Parser as StreamdownParser,
};
use streamdown_render::{
    fg_color, text_wrap, RenderFeatures, RenderStyle, Renderer as StreamdownRenderer, BULLETS,
};
use streamdown_syntax::Highlighter;

pub struct MarkdownRender {
    parser: StreamdownParser,
    width: usize,
    features: RenderFeatures,
    style: RenderStyle,
    computed_style: ComputedStyle,
    highlighter: Highlighter,
    code_language: Option<String>,
}

impl MarkdownRender {
    pub fn init(options: RenderOptions) -> Result<Self> {
        let streamdown = options.streamdown;
        let width = if streamdown.style.width > 0 {
            streamdown.style.width
        } else {
            match options.wrap.as_deref() {
                None | Some("auto") => terminal::size()
                    .map(|(columns, _)| columns as usize)
                    .unwrap_or(80),
                Some(value) => {
                    let max_width = value
                        .parse::<usize>()
                        .map_err(|_| anyhow!("Invalid wrap value"))?;
                    terminal::size()
                        .map(|(columns, _)| (columns as usize).min(max_width))
                        .unwrap_or(max_width)
                }
            }
        };

        let features = RenderFeatures {
            fixed_width: Some(width),
            pretty_pad: streamdown.style.pretty_pad,
            pretty_broken: options.wrap_code || streamdown.style.pretty_broken,
            clipboard: streamdown.features.clipboard,
            savebrace: streamdown.features.savebrace,
            width_wrap: width == 0,
            margin: streamdown.style.margin,
            ..Default::default()
        };
        let computed_style = streamdown.computed_style();
        let style = render_style_from_computed(&computed_style);

        let mut parser = StreamdownParser::new();
        parser.set_code_spaces(streamdown.features.code_spaces);
        parser.set_process_images(streamdown.features.images);
        parser.set_process_links(streamdown.features.links);

        let mut highlighter = Highlighter::new();
        if streamdown.style.syntax != "native" {
            highlighter.set_theme(&streamdown.style.syntax);
        }
        highlighter.set_background(parse_rgb(&computed_style.dark));

        Ok(Self {
            parser,
            width,
            features,
            style,
            computed_style,
            highlighter,
            code_language: None,
        })
    }

    pub fn render(&mut self, text: &str) -> String {
        self.render_text(text, true)
    }

    pub fn render_stream_text(&mut self, text: &str) -> String {
        self.render_text(text, false)
    }

    pub fn render_line(&self, line: &str) -> String {
        if self.code_language.is_some() {
            return self.render_code_line(line);
        }

        let mut parser = StreamdownParser::with_state(self.parser.state().clone());
        let events = parser.parse_line(line);
        self.preview_events(events)
    }

    fn render_text(&mut self, text: &str, finalize: bool) -> String {
        let mut events = Vec::new();
        for line in text.split('\n') {
            events.extend(self.parser.parse_line(line));
        }
        if finalize {
            events.extend(self.parser.finalize());
        }
        self.render_events(events)
    }

    fn render_events(&mut self, events: Vec<ParseEvent>) -> String {
        let mut output = Vec::new();
        let mut standard_events = Vec::new();

        for event in events {
            match event {
                ParseEvent::CodeBlockStart { language, .. } => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    self.render_code_label(&mut output, language.as_deref());
                    self.code_language = language;
                }
                ParseEvent::CodeBlockLine(line) => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    let line = self.render_code_line(&line);
                    output.extend_from_slice(line.as_bytes());
                    output.push(b'\n');
                }
                ParseEvent::CodeBlockEnd => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    self.code_language = None;
                }
                event @ ParseEvent::Heading { .. } | event @ ParseEvent::ListItem { .. } => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    let line = self.render_custom_block(&event);
                    output.extend_from_slice(line.as_bytes());
                    output.push(b'\n');
                }
                event
                    if event.is_inline()
                        || matches!(
                            event,
                            ParseEvent::InlineElements(_)
                                | ParseEvent::Newline
                                | ParseEvent::EmptyLine
                        ) =>
                {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    let text = self.render_inline_event(&event);
                    output.extend_from_slice(text.as_bytes());
                }
                event => standard_events.push(event),
            }
        }

        self.render_standard_events(&mut output, &standard_events);

        String::from_utf8_lossy(&output)
            .trim_end_matches(['\r', '\n'])
            .to_string()
    }

    fn render_standard_events(&self, output: &mut Vec<u8>, events: &[ParseEvent]) {
        if events.is_empty() {
            return;
        }
        let mut renderer = self.renderer(output);
        let _ = renderer.render(events);
    }

    fn preview_events(&self, events: Vec<ParseEvent>) -> String {
        let mut output = Vec::new();
        let mut standard_events = Vec::new();
        for event in events {
            match event {
                ParseEvent::CodeBlockStart { language, .. } => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    self.render_code_label(&mut output, language.as_deref());
                }
                ParseEvent::CodeBlockLine(line) => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    let line =
                        self.render_code_line_with_language(&line, self.code_language.as_deref());
                    output.extend_from_slice(line.as_bytes());
                    output.push(b'\n');
                }
                ParseEvent::CodeBlockEnd => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                }
                event @ ParseEvent::Heading { .. } | event @ ParseEvent::ListItem { .. } => {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    let line = self.render_custom_block(&event);
                    output.extend_from_slice(line.as_bytes());
                    output.push(b'\n');
                }
                event
                    if event.is_inline()
                        || matches!(
                            event,
                            ParseEvent::InlineElements(_)
                                | ParseEvent::Newline
                                | ParseEvent::EmptyLine
                        ) =>
                {
                    self.render_standard_events(&mut output, &standard_events);
                    standard_events.clear();
                    let text = self.render_inline_event(&event);
                    output.extend_from_slice(text.as_bytes());
                }
                event => standard_events.push(event),
            }
        }
        self.render_standard_events(&mut output, &standard_events);
        String::from_utf8_lossy(&output)
            .trim_end_matches(['\r', '\n'])
            .to_string()
    }

    fn render_custom_block(&self, event: &ParseEvent) -> String {
        match event {
            ParseEvent::Heading { level, content } => self.render_heading(*level, content),
            ParseEvent::ListItem {
                indent,
                bullet,
                content,
            } => self.render_list_item(*indent, bullet, content),
            _ => String::new(),
        }
    }

    fn render_heading(&self, level: u8, content: &str) -> String {
        let content = self.render_inline_content(content);
        let color = match level {
            1 | 2 => &self.style.bright,
            3 => &self.style.head,
            4 => &self.style.symbol,
            5 => &self.style.grey,
            _ => &self.style.grey,
        };
        let fg = fg_color(color);
        let visible = visible_length(&content);
        let left = if level <= 2 {
            " ".repeat(self.width.saturating_sub(visible) / 2)
        } else {
            String::new()
        };
        format!("\x1b[1m{fg}{left}{content}\x1b[22m\x1b[0m")
    }

    fn render_list_item(&self, indent: usize, bullet: &ListBullet, content: &str) -> String {
        let marker = match bullet {
            ListBullet::Ordered(num) => format!("{num}."),
            ListBullet::PlusExpand => "⊞".to_string(),
            _ => {
                let level = indent / 2;
                BULLETS[level % BULLETS.len()].to_string()
            }
        };
        let indent_spaces = indent * 2;
        let marker_width = visible_length(&marker);
        let content_indent = indent_spaces + marker_width + 1;
        let marker = format!("{}{}{}", fg_color(&self.style.symbol), marker, "\x1b[0m");
        let content = self.render_inline_content(content);
        let first_prefix = format!("{}{} ", " ".repeat(indent_spaces), marker);
        let next_prefix = " ".repeat(content_indent);
        let content_width = self.width.saturating_sub(content_indent);
        text_wrap(
            &content,
            content_width,
            0,
            &first_prefix,
            &next_prefix,
            false,
            true,
        )
        .lines
        .join("\n")
    }

    fn render_inline_content(&self, content: &str) -> String {
        let mut parser = InlineParser::new();
        parser
            .parse(content)
            .into_iter()
            .map(|element| match element {
                InlineElement::Text(text) => text,
                InlineElement::Bold(text) => format!("\x1b[1m{text}\x1b[22m"),
                InlineElement::Italic(text) => format!("\x1b[3m{text}\x1b[23m"),
                InlineElement::BoldItalic(text) => format!("\x1b[1m\x1b[3m{text}\x1b[23m\x1b[22m"),
                InlineElement::Underline(text) => format!("\x1b[4m{text}\x1b[24m"),
                InlineElement::Strikeout(text) => format!("\x1b[9m{text}\x1b[29m"),
                InlineElement::Code(text) => {
                    format!("{}{}\x1b[0m", fg_color(&self.style.symbol), text)
                }
                InlineElement::Link { text, url } => {
                    format!(
                        "\x1b[4m{text}\x1b[24m {}({})\x1b[0m",
                        fg_color(&self.style.grey),
                        url
                    )
                }
                InlineElement::Image { alt, .. } => {
                    format!("{}[{}]\x1b[0m", fg_color(&self.style.symbol), alt)
                }
                InlineElement::Footnote(text) => {
                    format!("{}{}\x1b[0m", fg_color(&self.style.symbol), text)
                }
            })
            .collect()
    }

    fn render_inline_event(&self, event: &ParseEvent) -> String {
        match event {
            ParseEvent::Text(text) => text.clone(),
            ParseEvent::InlineCode(text) => {
                format!("{}{}\x1b[0m", fg_color(&self.style.symbol), text)
            }
            ParseEvent::Bold(text) => format!("\x1b[1m{text}\x1b[22m"),
            ParseEvent::Italic(text) => format!("\x1b[3m{text}\x1b[23m"),
            ParseEvent::BoldItalic(text) => format!("\x1b[1m\x1b[3m{text}\x1b[23m\x1b[22m"),
            ParseEvent::Underline(text) => format!("\x1b[4m{text}\x1b[24m"),
            ParseEvent::Strikeout(text) => format!("\x1b[9m{text}\x1b[29m"),
            ParseEvent::Link { text, url } => {
                format!("\x1b]8;;{url}\x1b\\\x1b[4m{text}\x1b[24m\x1b]8;;\x1b\\")
            }
            ParseEvent::Image { alt, .. } => {
                format!("{}[{}]\x1b[0m", fg_color(&self.style.symbol), alt)
            }
            ParseEvent::Footnote(text) => {
                format!("{}{}\x1b[0m", fg_color(&self.style.symbol), text)
            }
            ParseEvent::InlineElements(elements) => elements
                .iter()
                .map(|element| match element {
                    InlineElement::Text(text) => text.clone(),
                    InlineElement::Bold(text) => format!("\x1b[1m{text}\x1b[22m"),
                    InlineElement::Italic(text) => format!("\x1b[3m{text}\x1b[23m"),
                    InlineElement::BoldItalic(text) => {
                        format!("\x1b[1m\x1b[3m{text}\x1b[23m\x1b[22m")
                    }
                    InlineElement::Underline(text) => format!("\x1b[4m{text}\x1b[24m"),
                    InlineElement::Strikeout(text) => format!("\x1b[9m{text}\x1b[29m"),
                    InlineElement::Code(text) => {
                        format!("{}{}\x1b[0m", fg_color(&self.style.symbol), text)
                    }
                    InlineElement::Link { text, url } => {
                        format!("\x1b]8;;{url}\x1b\\\x1b[4m{text}\x1b[24m\x1b]8;;\x1b\\")
                    }
                    InlineElement::Image { alt, .. } => {
                        format!("{}[{}]\x1b[0m", fg_color(&self.style.symbol), alt)
                    }
                    InlineElement::Footnote(text) => {
                        format!("{}{}\x1b[0m", fg_color(&self.style.symbol), text)
                    }
                })
                .collect(),
            ParseEvent::Newline | ParseEvent::EmptyLine => "\n".to_string(),
            _ => String::new(),
        }
    }

    fn renderer<'a>(&self, output: &'a mut Vec<u8>) -> StreamdownRenderer<&'a mut Vec<u8>> {
        let mut renderer = StreamdownRenderer::with_style(output, self.width, self.style.clone());
        renderer.set_features(self.features.clone());
        renderer
    }

    fn render_code_label(&self, output: &mut Vec<u8>, language: Option<&str>) {
        let Some(language) = language.filter(|v| !v.is_empty() && *v != "text") else {
            return;
        };

        let bg = code_bg(&self.computed_style);
        let symbol = code_symbol(&self.computed_style);
        let reset = "\x1b[0m";
        let label = format!("{bg}{symbol}[{language}]{reset}");
        let padding = self.width.saturating_sub(visible_length(&label));
        output.extend_from_slice(format!("{label}{bg}{}{reset}\n", " ".repeat(padding)).as_bytes());
    }

    fn render_code_line(&self, line: &str) -> String {
        self.render_code_line_with_language(line, self.code_language.as_deref())
    }

    fn render_code_line_with_language(&self, line: &str, language: Option<&str>) -> String {
        let bg = code_bg(&self.computed_style);
        let reset = "\x1b[0m";
        let highlighted = self
            .highlighter
            .highlight(line, language)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        let padding = self.width.saturating_sub(visible_length(&highlighted));
        format!("{bg}{highlighted}{bg}{}{reset}", " ".repeat(padding))
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub wrap: Option<String>,
    pub wrap_code: bool,
    pub streamdown: StreamdownConfig,
}

impl RenderOptions {
    pub(crate) fn new(
        _theme: Option<syntect::highlighting::Theme>,
        wrap: Option<String>,
        wrap_code: bool,
        _truecolor: bool,
        streamdown: StreamdownConfig,
    ) -> Self {
        Self {
            wrap,
            wrap_code,
            streamdown,
        }
    }
}

fn code_bg(style: &ComputedStyle) -> String {
    format!("\x1b[48;2;{}", style.dark)
}

fn code_symbol(style: &ComputedStyle) -> String {
    format!("\x1b[38;2;{}", style.symbol)
}

fn parse_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let value = value.trim_end_matches('m');
    let mut parts = value.split(';');
    let r = parts.next()?.parse().ok()?;
    let g = parts.next()?.parse().ok()?;
    let b = parts.next()?.parse().ok()?;
    Some((r, g, b))
}

fn render_style_from_computed(style: &ComputedStyle) -> RenderStyle {
    RenderStyle {
        bright: rgb_hex(&style.bright),
        head: rgb_hex(&style.head),
        symbol: rgb_hex(&style.symbol),
        grey: rgb_hex(&style.grey),
        dark: rgb_hex(&style.dark),
        mid: rgb_hex(&style.mid),
        light: rgb_hex(&style.mid),
    }
}

fn rgb_hex(value: &str) -> String {
    parse_rgb(value)
        .map(|(r, g, b)| format!("#{r:02x}{g:02x}{b:02x}"))
        .unwrap_or_else(|| "#808080".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"
To unzip a file in Rust, you can use the `zip` crate. Here's an example code that shows how to unzip a file:

```rust
use std::fs::File;

fn unzip_file(path: &str, output_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    todo!()
}
```
"#;

    #[test]
    fn renders_markdown_with_streamdown() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render(TEXT);
        assert!(output.contains("zip"));
        assert!(output.contains("File"));
    }

    #[test]
    fn renders_stream_text_without_finalizing_open_blocks() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render_stream_text("```rust\nlet x = 1;");
        assert!(output.contains(" x "));
        assert!(output.contains("1"));
        assert!(!output.contains("```"));
        assert!(output.contains("[rust]"));
        assert!(!output.contains("▄"));
        assert!(!output.contains("▀"));
    }

    #[test]
    fn previews_code_line_without_streamdown_frame() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let _ = render.render_stream_text("```rust\n");
        let output = render.render_line("let x = 1;");
        assert!(output.contains(" x "));
        assert!(output.contains("1"));
        assert!(!output.contains("▄"));
        assert!(!output.contains("▀"));
    }

    #[test]
    fn code_line_background_extends_to_width() {
        let options = RenderOptions {
            streamdown: StreamdownConfig {
                style: streamdown_config::StyleConfig {
                    width: 40,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let mut render = MarkdownRender::init(options).unwrap();
        let _ = render.render_stream_text("```rust\n");
        let output = render.render_line("let x = 1;");
        assert!(visible_length(&output) >= 40);
        assert!(output.contains("\x1b[48;2;"));
    }

    #[test]
    fn previews_single_line() {
        let options = RenderOptions::default();
        let render = MarkdownRender::init(options).unwrap();
        let output = render.render_line("hello `rust`");
        assert!(output.contains("hello"));
        assert!(output.contains("rust"));
        assert!(!output.ends_with('\n'));
    }

    #[test]
    fn ordered_lists_keep_parser_numbers() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render("1. first\n1. second\n1. third");
        assert!(output.contains("1."));
        assert!(output.contains("2."));
        assert!(output.contains("3."));
    }

    #[test]
    fn headings_and_list_markers_are_styled() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render("# Title\n\n- item");
        assert!(output.contains("Title"));
        assert!(output.contains("•"));
        assert!(output.contains("\x1b[38;2;"));
    }

    #[test]
    fn non_code_blocks_do_not_use_background_color() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render("# Title\n\ntext `inline`\n\n- item\n\n1. ordered");
        assert!(!output.contains("\x1b[48;2;"));

        let output = render.render("\n```rust\nlet x = 1;\n```");
        assert!(output.contains("\x1b[48;2;"));
    }
}
