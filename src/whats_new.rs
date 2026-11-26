//! The What's new window: after an update, what changed in every version
//! since the one last used here. The text is `RELEASE_NOTES.md`, compiled
//! in, so it needs no connection and is kept in one place.
use crate::{
    Dudel, egui,
    update::{Version, version},
};

const NOTES: &str = include_str!("../RELEASE_NOTES.md");
const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// The settings of 0.6.2 and earlier do not say which version saved them:
/// show them everything from 0.6.0 on.
const BEFORE_RECORDED: Version = [0, 5, u32::MAX];
const WIDTH: f32 = 560.0;

#[derive(Default)]
pub(crate) struct WhatsNew {
    pub open: bool,
    /// The newest version whose changes were shown here (a preference);
    /// empty until one is recorded.
    pub seen: String,
    /// Show the versions after this one.
    since: Version,
    /// The welcome window follows, at startup.
    then_welcome: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block {
    Heading(String),
    Paragraph(String),
    Bullet(String),
}

/// One version's notes: what to show at once, and the rest.
struct Entry {
    version: Version,
    title: String,
    brief: Vec<Block>,
    details: Vec<Block>,
}

/// The Markdown the release notes use: headings, paragraphs and bullets,
/// wrapped over lines. Images are left out.
pub(crate) fn blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut continues = false;
    for line in text.lines() {
        let indented = line.starts_with(' ');
        let line = line.trim();
        if line.is_empty() || line.starts_with("![") {
            continues = false;
        } else if line.starts_with('#') {
            blocks.push(Block::Heading(line.trim_start_matches('#').trim().into()));
            continues = false;
        } else if let Some(item) = line.strip_prefix("- ") {
            blocks.push(Block::Bullet(item.into()));
            continues = true;
        } else {
            match blocks.last_mut() {
                Some(Block::Paragraph(text)) if continues => {
                    text.push(' ');
                    text.push_str(line);
                }
                Some(Block::Bullet(text)) if continues || indented => {
                    text.push(' ');
                    text.push_str(line);
                }
                _ => blocks.push(Block::Paragraph(line.into())),
            }
            continues = true;
        }
    }
    blocks
}

/// The versions in `notes` after `since`, up to `current`, newest first.
fn entries(notes: &str, since: Version, current: Version) -> Vec<Entry> {
    let mut entries: Vec<Entry> = notes
        .split("\n## ")
        .skip(1)
        .filter_map(|section| {
            let (title, body) = section.split_once('\n')?;
            let number = version(title.split_whitespace().next()?)?;
            (number > since && number <= current).then(|| {
                let (brief, details) = split(blocks(body));
                Entry {
                    version: number,
                    title: title.trim().to_owned(),
                    brief,
                    details,
                }
            })
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.version));
    entries
}

/// The introduction and the Highlights section first, if there is one;
/// otherwise everything before the first section.
fn split(blocks: Vec<Block>) -> (Vec<Block>, Vec<Block>) {
    let is_highlights = |block: &Block| matches!(block, Block::Heading(h) if h == "Highlights");
    let highlighted = blocks.iter().any(is_highlights);
    let (mut brief, mut details) = (Vec::new(), Vec::new());
    let mut section = None;
    for block in blocks {
        if matches!(block, Block::Heading(_)) {
            let highlights = is_highlights(&block);
            section = Some(highlights);
            if highlights {
                continue;
            }
        }
        let first = match section {
            Some(highlights) => highlights,
            None => !highlighted || matches!(block, Block::Paragraph(_)),
        };
        if first { &mut brief } else { &mut details }.push(block);
    }
    (brief, details)
}

/// A release page's notes without how to install it; of a release that was
/// skipped over, without its advice either.
pub(crate) fn published(body: &str, newest: bool) -> Vec<Block> {
    let body: String = body.chars().take(20_000).collect();
    let mut blocks = blocks(&body);
    let end = blocks.iter().position(|block| {
        matches!(block, Block::Heading(h) if h == "Install" || (!newest && h == "Good to know"))
    });
    blocks.truncate(end.unwrap_or(blocks.len()));
    blocks
}

/// What a run of inline text is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Look {
    strong: bool,
    italic: bool,
    code: bool,
}

/// Bold, italic and code runs. Links keep their text only: some of this
/// text comes from the release page.
fn runs(text: &str) -> Vec<(String, Look)> {
    let mut runs: Vec<(String, Look)> = Vec::new();
    let mut look = Look::default();
    let mut push = |c: char, look: Look| match runs.last_mut() {
        Some((text, last)) if *last == look => text.push(c),
        _ => runs.push((c.into(), look)),
    };
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '`' {
            look.code = !look.code;
        } else if !look.code && rest.starts_with("**") {
            look.strong = !look.strong;
            rest = &rest[1..];
        } else if !look.code && c == '*' {
            look.italic = !look.italic;
        } else if !look.code
            && c == '['
            && let Some((label, after)) = rest[1..].split_once("](")
            && !label.contains(']')
            && let Some((_, after)) = after.split_once(')')
        {
            for c in label.chars() {
                push(c, look);
            }
            rest = after;
            continue;
        } else {
            push(c, look);
        }
        rest = &rest[c.len_utf8()..];
    }
    runs
}

fn text(ui: &egui::Ui, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let visuals = ui.visuals();
    let style = |style| egui::TextStyle::resolve(&style, ui.style());
    for (text, look) in runs(text) {
        let mut format = egui::TextFormat::simple(
            style(if look.code {
                egui::TextStyle::Monospace
            } else {
                egui::TextStyle::Body
            }),
            if look.strong {
                visuals.strong_text_color()
            } else {
                visuals.text_color()
            },
        );
        format.italics = look.italic;
        job.append(&text, 0.0, format);
    }
    job
}

pub(crate) fn show_blocks(ui: &mut egui::Ui, blocks: &[Block]) {
    for block in blocks {
        match block {
            Block::Heading(heading) => {
                ui.add_space(6.0);
                ui.strong(heading);
            }
            Block::Paragraph(paragraph) => {
                let job = text(ui, paragraph);
                ui.add(egui::Label::new(job).wrap());
            }
            Block::Bullet(item) => {
                ui.horizontal_top(|ui| {
                    ui.label("•");
                    let job = text(ui, item);
                    ui.add(egui::Label::new(job).wrap());
                });
            }
        }
    }
}

impl Dudel {
    /// At startup, after the preferences are read. `first_start`: nothing
    /// was saved here before. `hold`: something else needs the artist first;
    /// the window waits for a later start.
    pub(crate) fn start_whats_new(&mut self, first_start: bool, hold: bool) {
        let current = version(CURRENT).unwrap_or_default();
        let pretend = crate::identity::DEVELOPMENT
            .then(|| std::env::var("DUDEL_UPDATED_FROM").ok())
            .flatten();
        let seen = version(pretend.as_deref().unwrap_or(&self.whats_new.seen));
        if first_start && pretend.is_none() {
            self.whats_new.seen = CURRENT.to_owned();
            return;
        }
        let since = seen.unwrap_or(BEFORE_RECORDED);
        if hold || since >= current {
            return;
        }
        self.whats_new.seen = CURRENT.to_owned();
        if !entries(NOTES, since, current).is_empty() {
            self.whats_new.since = since;
            self.whats_new.then_welcome = std::mem::take(&mut self.welcome_open);
            self.whats_new.open = true;
        }
    }

    /// Help > What's new: every version, this one first.
    pub(crate) fn open_whats_new(&mut self) {
        self.whats_new.since = [0; 3];
        self.whats_new.then_welcome = false;
        self.whats_new.open = true;
    }

    /// What's new as it looks after updating from `from`, for screenshots.
    pub(crate) fn whats_new_capture_fixture(&mut self, from: Version) {
        self.welcome_open = false;
        self.whats_new.since = from;
        self.whats_new.open = true;
    }

    pub(crate) fn whats_new_ui(&mut self, ctx: &egui::Context) {
        if !self.whats_new.open {
            return;
        }
        let since = self.whats_new.since;
        let entries = entries(NOTES, since, version(CURRENT).unwrap_or_default());
        let mut close = false;
        let id = egui::Id::new("whats new");
        let room = crate::dialogs::dialog_body_room(ctx, id, 110.0).min(520.0);
        let mut body = 0.0;
        let response = crate::dialogs::modal(ctx, id).show(ctx, |ui| {
            ui.set_width(WIDTH);
            close |= crate::dialogs::dialog_title(ui, "What's new in Dudel");
            if since != [0; 3] {
                ui.label(format!("Dudel was updated to {CURRENT}."));
                ui.add_space(4.0);
            }
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            body = egui::ScrollArea::vertical()
                .max_height(room)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (index, entry) in entries.iter().enumerate() {
                        if index > 0 {
                            ui.add_space(8.0);
                            ui.separator();
                        }
                        ui.heading(&entry.title);
                        show_blocks(ui, &entry.brief);
                        if !entry.details.is_empty() {
                            egui::CollapsingHeader::new("Show everything")
                                .id_salt(entry.version)
                                .show(ui, |ui| show_blocks(ui, &entry.details));
                        }
                    }
                })
                .inner_rect
                .height();
            crate::dialogs::dialog_buttons_beside(
                ui,
                |ui| {
                    if since != [0; 3] {
                        ui.weak("Help › What's new shows this again.");
                    }
                },
                |ui| {
                    close |= ui
                        .add(crate::workspace::primary_button(ui, "Close"))
                        .clicked();
                },
            );
        });
        crate::dialogs::measure_dialog(ctx, id, response.response.rect.height(), body);
        if close || response.should_close() {
            self.whats_new.open = false;
            self.welcome_open |= std::mem::take(&mut self.whats_new.then_welcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Notes\n\n## Unreleased\n\n- Later.\n\n## 0.7.0 (a day)\n\nBig\nrelease.\n\n### Highlights\n\n- **One:** first\n  line.\n\n### Details\n\n- Two.\n\n## 0.6.2 (a day)\n\nA fix.\n\n- Fixed.\n\n### Limits\n\n- Some.\n\n## 0.6.0 (a day)\n\nOld.\n\n## Known limits\n\n- None.\n";

    fn titles(since: &str, current: &str) -> Vec<String> {
        entries(SAMPLE, version(since).unwrap(), version(current).unwrap())
            .into_iter()
            .map(|entry| entry.title.split(' ').next().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn every_skipped_version_is_listed_newest_first_and_no_later_one() {
        assert_eq!(titles("0.6.0", "0.7.0"), ["0.7.0", "0.6.2"]);
        assert_eq!(titles("0.5.0", "0.6.2"), ["0.6.2", "0.6.0"]);
        assert!(titles("0.7.0", "0.7.0").is_empty());
    }

    #[test]
    fn highlights_come_first_and_the_rest_is_kept() {
        let all = entries(SAMPLE, [0; 3], [0, 7, 0]);
        assert_eq!(
            all[0].brief,
            [
                Block::Paragraph("Big release.".into()),
                Block::Bullet("**One:** first line.".into()),
            ]
        );
        assert_eq!(
            all[0].details,
            [
                Block::Heading("Details".into()),
                Block::Bullet("Two.".into())
            ]
        );
        // Without highlights, what precedes the first section.
        assert_eq!(all[1].brief.len(), 2);
        assert_eq!(all[1].details.len(), 2);
    }

    #[test]
    fn inline_marks_become_looks_and_links_keep_their_text() {
        let strong = Look {
            strong: true,
            ..Look::default()
        };
        assert_eq!(
            runs("**A** in [the notes](https://x.y/z) é"),
            [
                ("A".into(), strong),
                (" in the notes é".into(), Look::default())
            ]
        );
        assert_eq!(runs("`a * b`")[0].0, "a * b");
        assert_eq!(runs("[open")[0].0, "[open");
    }

    #[test]
    fn a_release_page_is_shown_without_its_installing_section() {
        let body = "Intro\n\n![x](y)\n\n## What's new\n\n- A\n\n## Good to know\n\n- B\n\n## Install\n\n1. C";
        assert_eq!(published(body, true).len(), 5);
        assert_eq!(published(body, false).len(), 3);
    }

    /// Written so a release cannot be built without notes to show.
    #[test]
    fn the_release_notes_describe_this_version() {
        let current = version(CURRENT).unwrap();
        let all = entries(NOTES, [0; 3], current);
        assert_eq!(all[0].version, current, "RELEASE_NOTES.md lacks {CURRENT}");
        assert!(all.iter().all(|entry| !entry.brief.is_empty()));
        assert!(entries(NOTES, BEFORE_RECORDED, current).len() >= 3);
    }

    #[test]
    fn it_opens_once_after_an_update_and_never_on_a_new_install() {
        let mut app = Dudel::default();
        app.start_whats_new(true, false);
        assert!(!app.whats_new.open);
        assert_eq!(app.whats_new.seen, CURRENT);
        // Settings from before versions were recorded; recovery comes first.
        let mut app = Dudel {
            welcome_open: true,
            ..Dudel::default()
        };
        app.start_whats_new(false, true);
        assert!(!app.whats_new.open && app.whats_new.seen.is_empty());
        app.start_whats_new(false, false);
        assert!(app.whats_new.open && !app.welcome_open);
        // The next start, with what was saved.
        let mut next = Dudel::default();
        next.whats_new.seen = app.whats_new.seen.clone();
        next.start_whats_new(false, false);
        assert!(!next.whats_new.open);
        // An older copy run in between does not forget the newer one.
        next.whats_new.seen = "99.0.0".into();
        next.start_whats_new(false, false);
        assert!(!next.whats_new.open);
        assert_eq!(next.whats_new.seen, "99.0.0");
    }

    #[test]
    fn the_window_draws_every_version_and_stays_open_until_closed() {
        let mut app = Dudel::default();
        app.open_whats_new();
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run(Default::default(), |ctx| {
                crate::workspace::theme(ctx);
                app.whats_new_ui(ctx);
            });
        }
        assert!(app.whats_new.open);
    }
}
