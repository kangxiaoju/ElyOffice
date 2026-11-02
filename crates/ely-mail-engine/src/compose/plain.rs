use std::collections::HashMap;

use markdown::mdast::{Definition, Node};

pub(super) fn options() -> markdown::ParseOptions {
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.code_indented = false;
    options
}

pub(super) fn bodies(source: &str) -> crate::Result<(String, String)> {
    let tree = markdown::to_mdast(source, &options())
        .map_err(|error| crate::Error::Mime(error.to_string()))?;
    let mut definitions = HashMap::new();
    collect_definitions(&tree, &mut definitions);
    let renderer = Renderer {
        source,
        definitions,
    };
    let plain = renderer.render(&tree, false);
    let html = format!(
        "<div style=\"white-space:pre-wrap\">{}</div>",
        renderer.render(&tree, true)
    );
    Ok((plain, html))
}

fn escaped(text: &str, html: bool) -> String {
    if html {
        super::escape(text)
    } else {
        text.to_owned()
    }
}

fn collect_definitions<'a>(node: &'a Node, definitions: &mut HashMap<&'a str, &'a Definition>) {
    if let Node::Definition(definition) = node {
        definitions
            .entry(&definition.identifier)
            .or_insert(definition);
    }
    if let Some(nodes) = node.children() {
        for node in nodes {
            collect_definitions(node, definitions);
        }
    }
}

struct Renderer<'a> {
    source: &'a str,
    definitions: HashMap<&'a str, &'a Definition>,
}

impl Renderer<'_> {
    fn children(&self, nodes: &[Node], html: bool, separator: &str) -> String {
        nodes
            .iter()
            .map(|node| self.render(node, html))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(separator)
    }

    // mdast decodes punctuation but trims physical line-edge whitespace. Recover
    // only whitespace from source positions, never Markdown punctuation or escapes.
    fn source_text(&self, node: &Node, value: &str) -> String {
        let Some(position) = node.position() else {
            return value.to_owned();
        };
        let raw = &self.source[position.start.offset..position.end.offset];
        let mut raw_lines = raw.split('\n');
        value
            .split('\n')
            .enumerate()
            .map(|(index, line)| {
                let raw = raw_lines.next().unwrap_or_default();
                if raw.chars().all(|ch| matches!(ch, ' ' | '\t'))
                    && line.trim_matches([' ', '\t']).is_empty()
                {
                    return raw.to_owned();
                }
                let prefix = if index == 0 {
                    ""
                } else {
                    &raw[..raw.len() - raw.trim_start_matches([' ', '\t']).len()]
                };
                let suffix = &raw[raw.trim_end_matches([' ', '\t']).len()..];
                let line = if index == 0 {
                    line
                } else {
                    line.trim_start_matches([' ', '\t'])
                };
                let line = if suffix.is_empty() {
                    line
                } else {
                    line.trim_end_matches([' ', '\t'])
                };
                format!("{prefix}{line}{suffix}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn positioned(&self, nodes: &[Node], start: usize, finish: usize, html: bool) -> String {
        let mut out = String::new();
        let mut end = start;
        for node in nodes {
            if let Some(position) = node.position() {
                let gap = &self.source[end..position.start.offset];
                if gap.chars().all(char::is_whitespace) {
                    out.push_str(&escaped(gap, html));
                }
                end = position.end.offset;
            }
            out.push_str(&self.render(node, html));
        }
        let tail = &self.source[end..finish];
        if tail.chars().all(char::is_whitespace) {
            out.push_str(&escaped(tail, html));
        }
        out
    }

    fn link(&self, body: String, url: &str, title: Option<&str>, html: bool) -> String {
        if !html {
            return body;
        }
        match crate::page::link(url) {
            Some(url) => format!(
                "<a href=\"{}\"{}>{body}</a>",
                super::escape(&url),
                Self::title(title)
            ),
            None => body,
        }
    }

    fn title(title: Option<&str>) -> String {
        title
            .map(|title| format!(" title=\"{}\"", super::escape(title)))
            .unwrap_or_default()
    }

    fn image(&self, alt: &str, url: &str, title: Option<&str>, html: bool) -> String {
        if html {
            if let Some(url) = crate::page::link(url).filter(|url| {
                !url.get(..7)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
            }) {
                return format!(
                    "<img src=\"{}\" alt=\"{}\"{}>",
                    super::escape(&url),
                    super::escape(alt),
                    Self::title(title)
                );
            }
        }
        escaped(alt, html)
    }

    fn render(&self, node: &Node, html: bool) -> String {
        let wrap = |tag: &str, body: String| {
            if html {
                format!("<{tag}>{body}</{tag}>")
            } else {
                body
            }
        };
        match node {
            Node::Root(root_node) => {
                self.positioned(&root_node.children, 0, self.source.len(), html)
            }
            Node::Text(text) => escaped(&self.source_text(node, &text.value), html),
            Node::Paragraph(paragraph) => match node.position() {
                Some(position) => {
                    let raw = &self.source[position.start.offset..position.end.offset];
                    let prefix = &raw[..raw.len() - raw.trim_start_matches([' ', '\t']).len()];
                    format!(
                        "{}{}",
                        escaped(prefix, html),
                        self.positioned(
                            &paragraph.children,
                            position.start.offset + prefix.len(),
                            position.end.offset,
                            html
                        )
                    )
                }
                None => self.children(&paragraph.children, html, ""),
            },
            Node::Heading(heading) => wrap(
                &format!("h{}", heading.depth),
                self.children(&heading.children, html, ""),
            ),
            Node::Strong(value) => wrap("strong", self.children(&value.children, html, "")),
            Node::Emphasis(value) => wrap("em", self.children(&value.children, html, "")),
            Node::Delete(value) => wrap("del", self.children(&value.children, html, "")),
            Node::InlineCode(value) => wrap("code", escaped(&value.value, html)),
            Node::Code(value) => wrap("pre", wrap("code", escaped(&value.value, html))),
            Node::Html(value) => escaped(&value.value, html),
            Node::Link(link) => self.link(
                self.children(&link.children, html, ""),
                &link.url,
                link.title.as_deref(),
                html,
            ),
            Node::LinkReference(link) => {
                let body = self.children(&link.children, html, "");
                match self.definitions.get(link.identifier.as_str()) {
                    Some(definition) => {
                        self.link(body, &definition.url, definition.title.as_deref(), html)
                    }
                    None => body,
                }
            }
            Node::Blockquote(quote) => {
                let body = self.children(&quote.children, html, "\n\n");
                if html {
                    wrap("blockquote", body)
                } else {
                    body.split('\n')
                        .map(|line| format!("> {line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            Node::List(list) => {
                let items = list
                    .children
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let Node::ListItem(item) = item else {
                            return self.render(item, html);
                        };
                        let state = match item.checked {
                            Some(true) => "[x] ",
                            Some(false) => "[ ] ",
                            None => "",
                        };
                        let body = self.children(&item.children, html, "\n\n");
                        if html {
                            let state = match item.checked {
                                Some(true) => "<input type=\"checkbox\" checked disabled> ",
                                Some(false) => "<input type=\"checkbox\" disabled> ",
                                None => "",
                            };
                            format!("<li>{state}{body}</li>")
                        } else {
                            let marker = if list.ordered {
                                format!("{}. ", u64::from(list.start.unwrap_or(1)) + index as u64)
                            } else {
                                "- ".into()
                            };
                            let mut lines = body.split('\n');
                            let mut out =
                                format!("{marker}{state}{}", lines.next().unwrap_or_default());
                            for line in lines {
                                out.push('\n');
                                if !line.is_empty() {
                                    out.push_str(&" ".repeat(marker.len()));
                                    out.push_str(line);
                                }
                            }
                            out
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(if html {
                        ""
                    } else if list.spread {
                        "\n\n"
                    } else {
                        "\n"
                    });
                if html {
                    if list.ordered {
                        format!("<ol start=\"{}\">{items}</ol>", list.start.unwrap_or(1))
                    } else {
                        wrap("ul", items)
                    }
                } else {
                    items
                }
            }
            Node::Table(table) => wrap(
                "table",
                self.children(&table.children, html, if html { "" } else { "\n" }),
            ),
            Node::TableRow(row) => wrap(
                "tr",
                self.children(&row.children, html, if html { "" } else { "\t" }),
            ),
            Node::TableCell(cell) => wrap("td", self.children(&cell.children, html, "")),
            Node::Break(_) => {
                let whitespace = node
                    .position()
                    .map(|position| {
                        let raw = &self.source[position.start.offset..position.end.offset];
                        // A backslash hard break is syntax, unlike actual trailing spaces.
                        if raw.starts_with('\\') {
                            ""
                        } else {
                            raw.trim_end_matches(['\r', '\n'])
                        }
                    })
                    .unwrap_or_default();
                format!(
                    "{}{}",
                    escaped(whitespace, html),
                    if html { "<br>" } else { "\n" }
                )
            }
            Node::ThematicBreak(_) => {
                if html {
                    "<hr>".into()
                } else {
                    "---".into()
                }
            }
            Node::Image(image) => self.image(&image.alt, &image.url, image.title.as_deref(), html),
            Node::ImageReference(image) => match self.definitions.get(image.identifier.as_str()) {
                Some(definition) => self.image(
                    &image.alt,
                    &definition.url,
                    definition.title.as_deref(),
                    html,
                ),
                None => escaped(&image.alt, html),
            },
            Node::Definition(_) | Node::FootnoteDefinition(_) => String::new(),
            other => match other.children() {
                Some(nodes) => self.children(nodes, html, ""),
                None => escaped(&other.to_string(), html),
            },
        }
    }
}
