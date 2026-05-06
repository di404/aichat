use anyhow::{anyhow, Result};
use crossterm::terminal;
use streamdown_parser::Parser as StreamdownParser;
use streamdown_render::{RenderFeatures, Renderer as StreamdownRenderer};

pub struct MarkdownRender {
    parser: StreamdownParser,
    width: usize,
    features: RenderFeatures,
}

impl MarkdownRender {
    pub fn init(options: RenderOptions) -> Result<Self> {
        let width = match options.wrap.as_deref() {
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
        };

        let features = RenderFeatures {
            fixed_width: Some(width),
            pretty_broken: options.wrap_code,
            margin: 0,
            ..Default::default()
        };

        Ok(Self {
            parser: StreamdownParser::new(),
            width,
            features,
        })
    }

    pub fn render(&mut self, text: &str) -> String {
        self.render_text(text, true)
    }

    pub fn render_stream_text(&mut self, text: &str) -> String {
        self.render_text(text, false)
    }

    pub fn render_line(&self, line: &str) -> String {
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
        let mut output = Vec::new();
        for line in text.split('\n') {
            let events = self.parser.parse_line(line);
            {
                let mut renderer = self.renderer(&mut output);
                let _ = renderer.render(&events);
            }
        }
        if finalize {
            let events = self.parser.finalize();
            let mut renderer = self.renderer(&mut output);
            let _ = renderer.render(&events);
        }
        String::from_utf8_lossy(&output)
            .trim_end_matches(['\r', '\n'])
            .to_string()
    }

    fn renderer<'a>(&self, output: &'a mut Vec<u8>) -> StreamdownRenderer<&'a mut Vec<u8>> {
        let mut renderer = StreamdownRenderer::new(output, self.width);
        renderer.set_features(self.features.clone());
        renderer
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub wrap: Option<String>,
    pub wrap_code: bool,
}

impl RenderOptions {
    pub(crate) fn new(
        _theme: Option<syntect::highlighting::Theme>,
        wrap: Option<String>,
        wrap_code: bool,
        _truecolor: bool,
    ) -> Self {
        Self { wrap, wrap_code }
    }
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
        assert!(output.contains("use std::fs::File;"));
    }

    #[test]
    fn renders_stream_text_without_finalizing_open_blocks() {
        let options = RenderOptions::default();
        let mut render = MarkdownRender::init(options).unwrap();
        let output = render.render_stream_text("```rust\nlet x = 1;");
        assert!(output.contains("let x = 1;"));
        assert!(!output.contains("```"));
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
