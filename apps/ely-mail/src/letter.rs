//! A message turned into what the pane draws (C12.10).

use ely_gpui_component::documents::{
    AlignKind, HeldPicture, Inline as Line, MarkdownBlock as Node, Pictures, Style as Mark,
};
use ely_mail_engine::{
    Part, Stored,
    page::{self, Block, Inline, Source, Style},
};
use gpui::{ImageFormat, SharedString};

/// Words drawn off the UI thread, read on it first.
#[derive(Clone)]
pub struct Words {
    pub broken: SharedString,
    pub waiting: SharedString,
    pub not_picture: SharedString,
    pub svg: SharedString,
    pub unnamed: SharedString,
}

/// A folded letter shows this many characters.
const SNIPPET: usize = 120;

/// One message as read; built off the UI thread.
pub struct Letter {
    /// The header block as it came.
    pub headers: String,
    /// None when only the headers are kept.
    pub body: Option<Body>,
}

/// What the store holds of a message.
pub enum Kept {
    Original(Vec<u8>),
    Head(Vec<u8>),
}

/// Blocks and pictures move into the pane's `Open`.
pub struct Body {
    pub blocks: Vec<Node>,
    pub pictures: Pictures,
    /// Web pictures shown, each address once.
    pub remote: Vec<String>,
    pub trackers: usize,
    pub short: bool,
    /// Its first words, for when it folds.
    pub snippet: String,
    /// Names, and sizes once downloaded.
    pub files: Vec<(String, Option<u64>)>,
}

pub fn letter(stored: &Stored, kept: Kept, words: &Words) -> Result<Letter, String> {
    let read = |bytes: &[u8]| ely_mail_engine::read(bytes).map_err(|err| err.to_string());
    let (body, headers) = match kept {
        Kept::Original(raw) => {
            let mail = read(&raw)?;
            let headers = mail.headers.clone();
            (Some(body(&stored.missing, mail, words)), headers)
        }
        Kept::Head(head) => (None, read(&head)?.headers),
    };
    Ok(Letter { headers, body })
}

pub fn body(missing: &[String], mut mail: ely_mail_engine::Mail, words: &Words) -> Body {
    let files = mail
        .attachments
        .iter()
        .map(|part| {
            let name = part
                .name
                .clone()
                .unwrap_or_else(|| words.unnamed.to_string());
            let waiting = part.bytes.is_empty()
                && (part.section.as_ref()).is_some_and(|at| missing.contains(at));
            (name, (!waiting).then_some(part.bytes.len() as u64))
        })
        .collect();
    // CID resources can carry generic or mislabeled MIME metadata. Keep download
    // rows; `held` validates actual raster bytes before any resource is drawn.
    mail.inline.extend(
        mail.attachments
            .into_iter()
            .filter(|part| part.content_id.is_some()),
    );
    let read = page::read(&mail.shown, &mail.inline);
    let snippet = read.snippet(SNIPPET);
    let mut held = Held {
        inline: &mail.inline,
        pictures: Pictures::new(),
        data: 0,
        words,
    };
    let blocks = held.nodes(read.blocks);
    Body {
        blocks,
        pictures: held.pictures,
        remote: read.remote,
        trackers: read.trackers,
        short: read.short,
        snippet,
        files,
    }
}

/// A picture's bytes as the pane holds them.
pub fn held(
    content_type: &str,
    bytes: Vec<u8>,
    words: &Words,
) -> Result<HeldPicture, SharedString> {
    // MIME parameters/case are metadata, never proof that the bytes are safe.
    // SVG may load files; mail never draws it, even under a raster MIME type.
    let kind = content_type.split(';').next().unwrap_or_default().trim();
    if kind.eq_ignore_ascii_case("image/svg+xml") {
        return Err(words.svg.clone());
    }
    if bytes.is_empty() {
        return Err(words.waiting.clone());
    }
    if bytes.len() as u64 > ely_mail_engine::fetch::LARGEST {
        return Err(words.broken.clone());
    }
    let detected = image::guess_format(&bytes).map_err(|_| words.not_picture.clone())?;
    let format = match detected {
        image::ImageFormat::Png => ImageFormat::Png,
        image::ImageFormat::Jpeg => ImageFormat::Jpeg,
        image::ImageFormat::Gif => ImageFormat::Gif,
        image::ImageFormat::Bmp => ImageFormat::Bmp,
        image::ImageFormat::WebP => ImageFormat::Webp,
        _ => return Err(words.not_picture.clone()),
    };
    // Both letter building and permitted remote fetches call this off-thread.
    // A signature alone can be truncated/corrupt; decode with explicit bounds
    // before handing the original bytes to the renderer's lazy asset cache.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 << 20);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), detected);
    reader.limits(limits);
    reader.decode().map_err(|_| words.broken.clone())?;
    Ok(HeldPicture::new(format, bytes))
}

/// The mail's pictures, keyed as blocks name them.
struct Held<'a> {
    inline: &'a [Part],
    pictures: Pictures,
    data: usize,
    words: &'a Words,
}

impl Held<'_> {
    fn nodes(&mut self, blocks: Vec<Block>) -> Vec<Node> {
        blocks.into_iter().map(|block| self.node(block)).collect()
    }

    fn node(&mut self, block: Block) -> Node {
        match block {
            Block::Text(inline) => Node::Paragraph(line(inline)),
            Block::Heading(level, inline) => Node::Heading(level, line(inline)),
            Block::Quote(inner) => Node::Quote(self.nodes(inner)),
            Block::List { start, items } => Node::List {
                start,
                items: (items.into_iter())
                    .map(|item| (None, self.nodes(item)))
                    .collect(),
            },
            Block::Table { head, rows, spans } => Node::Table {
                aligns: vec![AlignKind::None; head.len()],
                spans,
                head: head.into_iter().map(|cell| self.nodes(cell)).collect(),
                rows: rows
                    .into_iter()
                    .map(|row| row.into_iter().map(|cell| self.nodes(cell)).collect())
                    .collect(),
            },
            Block::Code(text) => Node::Code {
                language: None,
                text,
                from: 0,
            },
            Block::Rule => Node::Rule,
            Block::Image(image) => Node::Image {
                alt: image.alt,
                url: self.key(image.source),
                link: image.link,
                at: 0,
            },
        }
    }

    /// Web addresses stay as written: they wait.
    fn key(&mut self, source: Source) -> String {
        let (key, picture) = match source {
            Source::Remote(url) => return url,
            Source::Inline(at) => {
                let key = format!("ely-inline:{at}");
                if self.pictures.contains_key(&key) {
                    return key;
                }
                let part = &self.inline[at];
                (
                    key,
                    held(&part.content_type, part.bytes.clone(), self.words),
                )
            }
            Source::Data {
                content_type,
                bytes,
            } => {
                self.data += 1;
                let key = format!("ely-data:{}", self.data);
                (key, held(&content_type, bytes, self.words))
            }
            Source::Broken => ("ely-broken:".to_owned(), Err(self.words.broken.clone())),
        };
        self.pictures.insert(key.clone(), picture);
        key
    }
}

fn line(inline: Inline) -> Line {
    let mut styles: Vec<_> = (inline.styles.into_iter())
        .map(|(range, style)| {
            let mark = match style {
                Style::Strong => Mark::Strong,
                Style::Emphasis => Mark::Emphasis,
                Style::Strike => Mark::Strike,
                Style::Underline => Mark::Underline,
                Style::Code => Mark::Code,
            };
            (range, mark)
        })
        .collect();
    styles.extend(
        inline
            .links
            .iter()
            .map(|(range, _)| (range.clone(), Mark::Link)),
    );
    Line {
        text: inline.text,
        styles,
        links: inline.links,
        sources: Vec::new(),
    }
}
