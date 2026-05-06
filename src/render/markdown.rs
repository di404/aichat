use anyhow::{anyhow, Result};
use crossterm::terminal;
use streamdown_ansi::utils::visible_length;
use streamdown_config::{ComputedStyle, Config as StreamdownConfig};
use streamdown_parser::{ParseEvent, Parser as StreamdownParser};
use streamdown_render::{RenderFeatures, RenderStyle, Renderer as StreamdownRenderer};
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
        let style = RenderStyle::from_computed(&computed_style);

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

        let mut parser = StreamdownParser::new();
        let mut output = Vec::new();
        {
            let mut renderer = self.renderer(&mut output);
            let _ = renderer.render(&parser.parse_line(line));
        }
        String::from_utf8_lossy(&output)
            .trim_end_matches(['\r', '\n'])
            .to_string()
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
        let bg = code_bg(&self.computed_style);
        let reset = "\x1b[0m";
        let highlighted = self
            .highlighter
            .highlight(line, self.code_language.as_deref())
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
}
