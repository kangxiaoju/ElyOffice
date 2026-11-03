//! Background system rich clipboard and bounded inline picture import.

use std::io::{Cursor, Read as _};
use base64::Engine as _;
use ely_gpui_component::{documents::email::{EmailAlign, EmailDocument, EmailImage}, primitives::Severity};
use ely_mail_engine::Part;
use ely_platform::clipboard::{self, Clip};
use gpui::{AppContext as _, Context, PathPromptOptions};
use super::Compose;

const IMAGE_BYTES: usize = 16 * 1024 * 1024;
const IMAGE_EDGE: u32 = 8192;

fn picture(bytes: Vec<u8>) -> Result<(Part, EmailImage), String> {
    if bytes.len() > IMAGE_BYTES { return Err("Picture exceeds 16 MiB".into()); }
    let format = image::guess_format(&bytes).map_err(|error| error.to_string())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err("Only PNG and JPEG pictures are supported".into());
    }
    let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format).into_dimensions().map_err(|error| error.to_string())?;
    if width > IMAGE_EDGE || height > IMAGE_EDGE || u64::from(width) * u64::from(height) > 32 * 1024 * 1024 { return Err("Picture exceeds 8192 pixels per edge or 32 megapixels".into()); }
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(IMAGE_EDGE);
    limits.max_image_height = Some(IMAGE_EDGE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = format!("ely-{}-{}@inline", jiff::Timestamp::now().as_nanosecond(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let png = format == image::ImageFormat::Png;
    let part = Part { name: Some(if png { "image.png" } else { "image.jpg" }.into()), content_type: if png { "image/png" } else { "image/jpeg" }.into(), params: Vec::new(), content_id: Some(id.clone()), bytes, section: None };
    let image = EmailImage { source: format!("cid:{id}"), alt: "Picture".into(), width: Some(decoded.width()), height: Some(decoded.height()), align: EmailAlign::Left };
    Ok((part, image))
}

enum Pasted { Document(EmailDocument, Vec<Part>), Picture(Part, EmailImage), Empty }

fn import_html(html: &str) -> Result<Pasted, String> {
    let mut document = EmailDocument::from_html(html).map_err(|error| error.to_string())?;
    let mut parts = Vec::new();
    document.try_map_images(|image| {
        let remap = (|| -> Result<(), String> {
            if image.source.starts_with("data:") {
                let (header, data) = image.source.split_once(',').ok_or("Invalid inline picture")?;
                if !matches!(header, "data:image/png;base64" | "data:image/jpeg;base64") || data.len() > IMAGE_BYTES * 4 / 3 + 4 { return Err("Unsupported or oversized inline picture".into()); }
                let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|error| error.to_string())?;
                let (part, imported) = picture(bytes)?;
                image.source = imported.source;
                parts.push(part);
            } else if image.source.starts_with("cid:") {
                return Err("The clipboard picture has no embedded image bytes".into());
            }
            Ok(())
        })();
        remap.map_err(ely_gpui_component::documents::email::EmailError::Invalid)
    }).map_err(|error| error.to_string())?;
    Ok(Pasted::Document(document, parts))
}

impl Compose {
    /// Resumes a pending save/close, or cancels close when the host operation failed.
    pub(crate) fn finish_clipboard(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        match result {
            Err(error) => { self.editor_problem(error, cx); self.unread(cx); }
            Ok(()) => self.keep(cx),
        }
        cx.notify();
    }
    pub(super) fn remove_file(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        let cid = self.files.iter().find(|file| file.key.to_string() == key).and_then(|file| file.part.content_id.as_ref()).map(|id| format!("cid:{}", id.trim_matches(['<', '>'])));
        if let Some(cid) = cid {
            if self.document.read(cx).snapshot().images().iter().any(|image| image.source == cid) {
                self.editor_problem("This attachment is used by a picture in the message. Delete the picture first.".into(), cx);
                return;
            }
        }
        self.files.retain(|file| file.key.to_string() != key);
        self.sync_images(cx);
        self.touched(cx);
        cx.notify();
    }
    pub(super) fn sync_images(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<_> = self.files.iter().map(|file| file.key).collect();
        let parts: Vec<_> = self.files.iter().filter(|file| file.part.content_id.is_some()).map(|file| file.part.clone()).collect();
        if parts.is_empty() {
            self.document.update(cx, |editor, cx| editor.set_images(Default::default(), cx));
            return;
        }
        self.reading += 1;
        let job = cx.background_spawn(async move {
            parts.into_iter().map(|part| {
                let id = part.content_id.clone().ok_or("Picture has no CID")?;
                let (validated, _) = picture(part.bytes)?;
                let format = if validated.content_type == "image/png" { gpui::ImageFormat::Png } else { gpui::ImageFormat::Jpeg };
                Ok((format!("cid:{}", id.trim_matches(['<', '>'])), std::sync::Arc::new(gpui::Image::from_bytes(format, validated.bytes))))
            }).collect::<Result<std::collections::HashMap<_, _>, String>>()
        });
        cx.spawn(async move |this, cx| {
            let images = job.await;
            this.update(cx, |compose, cx| {
                compose.reading -= 1;
                if keys == compose.files.iter().map(|file| file.key).collect::<Vec<_>>() {
                    match images {
                        Ok(images) => compose.document.update(cx, |editor, cx| editor.set_images(images, cx)),
                        Err(error) => { compose.editor_problem(error, cx); compose.unread(cx); }
                    }
                }
                compose.touched(cx);
                cx.notify();
            }).ok();
        }).detach();
    }
    pub(super) fn editor_problem(&mut self, error: String, cx: &mut Context<Self>) {
        self.problem = Some((Severity::Danger, error.into()));
        cx.notify();
    }

    pub(super) fn copy(&mut self, html: String, text: String, cut: bool, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        let editor = self.document.read(cx);
        let (generation, revision, selection) = (editor.generation(), editor.document().revision(), editor.selection());
        let parts: Vec<_> = self.files.iter().filter(|file| file.part.content_id.is_some()).map(|file| file.part.clone()).collect();
        let job = cx.background_spawn(async move {
            let mut document = EmailDocument::from_html(&html).map_err(|error| error.to_string())?;
            document.try_map_images(|image| {
                let remap = (|| -> Result<(), String> {
                    if let Some(cid) = image.source.strip_prefix("cid:") {
                        let part = parts.iter().find(|part| part.content_id.as_deref().is_some_and(|id| id.trim_matches(['<', '>']) == cid)).ok_or("The selected picture has no attachment")?;
                        let (validated, _) = picture(part.bytes.clone())?;
                        image.source = format!("data:{};base64,{}", validated.content_type, base64::engine::general_purpose::STANDARD.encode(validated.bytes));
                    }
                    Ok(())
                })();
                remap.map_err(ely_gpui_component::documents::email::EmailError::Invalid)
            }).map_err(|error| error.to_string())?;
            let html = document.to_html();
            if html.len() > 2 * 1024 * 1024 { return Err("Rich clipboard exceeds 2 MiB".into()); }
            clipboard::write(&Clip::Html { html, text }).map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |compose, cx| match result {
                Err(error) => compose.editor_problem(error, cx),
                Ok(()) if cut => {
                    let editor = compose.document.read(cx);
                    if editor.generation() == generation && editor.document().revision() == revision && editor.selection() == selection {
                        let result = compose.document.update(cx, |editor, cx| editor.insert_text("", cx));
                        if let Err(error) = result { compose.editor_problem(error.to_string(), cx); }
                    }
                }
                Ok(()) => {}
            }).ok();
        }).detach();
    }

    pub(super) fn paste(&mut self, request: u64, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        self.reading += 1;
        let job = cx.background_spawn(async move {
            match clipboard::read().map_err(|error| error.to_string())? {
                Some(Clip::Html { html, .. }) => import_html(&html),
                Some(Clip::Text(text)) => EmailDocument::from_plain(&text).map(|document| Pasted::Document(document, Vec::new())).map_err(|error| error.to_string()),
                Some(Clip::Png(bytes)) => picture(bytes).map(|(part, image)| Pasted::Picture(part, image)),
                None => Ok(Pasted::Empty),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |compose, cx| {
                compose.reading -= 1;
                let result = match result {
                    Ok(Pasted::Document(document, parts)) => {
                        let applied = compose.document.update(cx, |editor, cx| editor.complete_paste(request, document, cx)).map_err(|error| error.to_string());
                        if applied.is_ok() && !parts.is_empty() {
                            for part in parts { compose.add_file(part); }
                            compose.sync_images(cx);
                            compose.touched(cx);
                        }
                        applied
                    }
                    Ok(Pasted::Picture(part, image)) => compose.finish_image(request, part, image, cx),
                    Ok(Pasted::Empty) => Ok(()),
                    Err(error) => Err(error),
                };
                compose.finish_clipboard(result, cx);
            }).ok();
        }).detach();
    }

    fn finish_image(&mut self, request: u64, part: Part, image: EmailImage, cx: &mut Context<Self>) -> Result<(), String> {
        if self.import_failed { return Err("The original draft could not be imported and cannot be edited".into()); }
        self.document.update(cx, |editor, cx| editor.complete_image(request, image, cx)).map_err(|error| error.to_string())?;
        self.add_file(part);
        self.sync_images(cx);
        self.touched(cx);
        Ok(())
    }

    pub(super) fn pick_image(&mut self, request: u64, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        let picked = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        self.reading += 1;
        cx.spawn(async move |this, cx| {
            let selected = picked.await;
            let job = cx.background_spawn(async move {
                let paths = selected.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else { return Ok(None); };
                let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
                let mut bytes = Vec::new();
                file.take(IMAGE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
                picture(bytes).map(Some)
            });
            let result = job.await;
            this.update(cx, |compose, cx| {
                compose.reading -= 1;
                let result = result.and_then(|picture| match picture { Some((part, image)) => compose.finish_image(request, part, image, cx), None => Ok(()) });
                compose.finish_clipboard(result, cx);
            }).ok();
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_svg_and_oversized_picture_before_decode() {
        assert!(picture(b"<svg/>".to_vec()).is_err());
        assert!(picture(vec![0; IMAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn embedded_clipboard_picture_becomes_fresh_cid_attachment() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255])).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let html = format!("<p><strong>Rich</strong> left <img src=\"data:image/png;base64,{}\" alt=\"photo\"> right</p>", base64::engine::general_purpose::STANDARD.encode(&png));
        let Pasted::Document(first, parts) = import_html(&html).unwrap() else { panic!("HTML document"); };
        let Pasted::Document(second, other) = import_html(&html).unwrap() else { panic!("HTML document"); };
        assert_eq!(parts[0].bytes, png);
        assert!(first.to_html().contains(&format!("cid:{}", parts[0].content_id.as_ref().unwrap())));
        assert_ne!(parts[0].content_id, other[0].content_id);
        assert!(second.to_html().contains("<strong>Rich</strong>"));
        assert_eq!(first.blocks().len(), 1, "the picture stays in its paragraph");
        assert_eq!(first.to_plain(), "Rich left photo right");
        assert_eq!(first.images().len(), 1);
        let selection = ely_gpui_component::documents::email::EmailSelection { anchor: first.first_point(), head: first.end_point() };
        let selected = first.fragment(&selection).unwrap();
        assert_eq!(selected.images().len(), 1);
        assert_eq!(selected.to_plain(), "Rich left photo right");
        let table_html = format!("<table><tr><td>left <img src=\"data:image/png;base64,{}\" alt=\"photo\"> right</td></tr></table>", base64::engine::general_purpose::STANDARD.encode(&png));
        let Pasted::Document(table, table_parts) = import_html(&table_html).unwrap() else { panic!("HTML table"); };
        assert_eq!(table.images().len(), 1);
        assert_eq!(table_parts.len(), 1);
        assert!(table.to_plain().contains("left photo right"));
    }

    #[test]
    fn inline_picture_bytes_and_cid_survive_mime_roundtrip() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255])).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (part, image) = picture(png.clone()).unwrap();
        let mailbox = |email: &str| ely_mail_engine::Mailbox { name: None, email: email.into() };
        let built = ely_mail_engine::build(&ely_mail_engine::Draft {
            from: mailbox("bo@example.com"), to: vec![mailbox("ann@example.com")], cc: Vec::new(), bcc: Vec::new(), subject: "Picture".into(),
            text: "Picture".into(), html: Some(format!("<img src=\"{}\" alt=\"Picture\">", image.source)), attachments: vec![part.clone()],
            in_reply_to: None, references: Vec::new(), date: 1_704_099_600,
        }).unwrap();
        let read = ely_mail_engine::read(&built.bytes).unwrap();
        assert!(read.html.as_ref().unwrap().contains(&image.source));
        let inline = read.inline.iter().find(|found| found.content_id == part.content_id).expect("sent CID image must be an inline MIME resource");
        assert_eq!(inline.bytes, png);
        let page = ely_mail_engine::page::read(&read.shown, &read.inline);
        assert!(matches!(&page.blocks[0], ely_mail_engine::page::Block::Image(image) if matches!(image.source, ely_mail_engine::page::Source::Inline(_))));
        assert!(read.attachments.is_empty());
    }

}
