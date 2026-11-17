//! What to paste when publishing an export (YouTube and the like): a title, a description that
//! ends with the chapter list, and tags. With AI on, Claude writes the title, description and
//! tags from the transcript (`sf_agent::details`); otherwise [`template`] fills them from the
//! project's name, the opening of the transcript and the chapter titles. The chapter list is
//! always added here, so its times are the export's own.

use crate::captions::Word;
use crate::timeline::Timeline;

/// Words left out of template tags.
const STOP: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "because", "been", "but", "by", "can", "could", "did", "do", "does", "doing", "don't", "for", "from", "get", "got", "going", "gonna", "had", "has", "have", "here", "how", "i", "i'm", "if", "in", "into", "is", "it", "it's", "its", "just", "know", "let's", "like", "make", "me", "more", "my", "new", "no", "not", "now", "of", "ok", "okay", "on", "one", "or", "our", "out", "really", "recording", "right", "see", "so", "some", "that", "that's", "the", "their", "them", "then", "there", "these", "they", "this", "to", "um", "uh", "untitled", "up", "us", "use", "very", "video", "want", "was", "we", "we're", "well", "were", "what", "when", "where", "which", "will", "with", "would", "yeah", "you", "you're", "your",
];

/// Most tags kept, and the longest description opening taken from the transcript.
pub const MAX_TAGS: usize = 12;
const OPENING_CHARS: usize = 280;

/// What an export's details are made from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Input {
    /// The project's name.
    pub name: String,
    /// What is said in the export, in order (cut words left out).
    pub transcript: String,
    /// Chapter titles in order (the export's own).
    pub chapters: Vec<String>,
    /// The chapter list for a YouTube description (`00:00 Intro` lines), or empty.
    pub chapter_list: String,
    pub duration_ms: f64,
}

/// A title, description and tags for publishing a video.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    pub title: String,
    /// Ends with the chapter list when there is one.
    pub description: String,
    pub tags: Vec<String>,
    /// Written by Claude (else by [`template`]).
    pub by_ai: bool,
}

impl Details {
    /// Tags as one comma-separated line (YouTube's tag field takes them pasted this way).
    pub fn tags_line(&self) -> String {
        self.tags.join(", ")
    }

    /// The file written beside an export.
    pub fn to_text(&self) -> String {
        let mut s = format!("Title\n{}\n\nDescription\n{}\n", self.title.trim(), self.description.trim());
        if !self.tags.is_empty() {
            s.push_str(&format!("\nTags\n{}\n", self.tags_line()));
        }
        s
    }
}

/// The words said between `start` and `end` of the edited timeline, in order, as plain text.
/// `words` are in source time; words in cut parts are left out.
pub fn transcript_text(words: &[Word], timeline: &Timeline, start: f64, end: f64) -> String {
    let mut placed: Vec<(f64, &str)> = words.iter().filter_map(|w| timeline.source_to_output(w.start_ms).filter(|t| *t >= start && *t < end).map(|t| (t, w.text.trim()))).filter(|(_, t)| !t.is_empty()).collect();
    placed.sort_by(|a, b| a.0.total_cmp(&b.0));
    placed.iter().map(|(_, t)| *t).collect::<Vec<_>>().join(" ")
}

/// `body` with the chapter list under it (a blank line between).
pub fn with_chapters(body: &str, chapter_list: &str) -> String {
    let body = body.trim();
    let list = chapter_list.trim();
    match (body.is_empty(), list.is_empty()) {
        (_, true) => body.to_string(),
        (true, false) => format!("Chapters\n{list}"),
        (false, false) => format!("{body}\n\nChapters\n{list}"),
    }
}

/// Details without AI: the project's name as the title, the first sentences said as the
/// description, and tags from the name, the chapter titles and words said often.
pub fn template(input: &Input) -> Details {
    let title = input.name.trim();
    let title = if title.is_empty() { "Untitled".to_string() } else { title.to_string() };
    let opening = opening(&input.transcript);
    Details { title, description: with_chapters(&opening, &input.chapter_list), tags: template_tags(input), by_ai: false }
}

/// Whole sentences from the start of `transcript`, up to about [`OPENING_CHARS`].
fn opening(transcript: &str) -> String {
    let t = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= OPENING_CHARS {
        return t;
    }
    let mut out = String::new();
    for sentence in t.split_inclusive(['.', '!', '?']) {
        if !out.is_empty() && out.chars().count() + sentence.chars().count() > OPENING_CHARS {
            break;
        }
        out.push_str(sentence);
    }
    if out.chars().count() > OPENING_CHARS + 40 {
        // one very long sentence: cut at a word and say there is more
        let mut cut: String = out.chars().take(OPENING_CHARS).collect();
        if let Some(i) = cut.rfind(' ') {
            cut.truncate(i);
        }
        out = format!("{}…", cut.trim_end_matches([',', ';', ':']));
    }
    out.trim().to_string()
}

fn words_of(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-' || c == '+' || c == '#')).map(|w| w.trim_matches(['\'', '-']).to_lowercase()).filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()))
}

fn template_tags(input: &Input) -> Vec<String> {
    let mut tags: Vec<String> = vec![];
    let push = |t: String, tags: &mut Vec<String>| {
        if tags.len() < MAX_TAGS && !tags.contains(&t) {
            tags.push(t);
        }
    };
    for w in words_of(&input.name) {
        push(w, &mut tags);
    }
    for c in &input.chapters {
        for w in words_of(c) {
            push(w, &mut tags);
        }
    }
    // words said at least three times, most often first (ties: first said first)
    let mut counts: Vec<(String, usize, usize)> = vec![];
    for (i, w) in words_of(&input.transcript).enumerate() {
        match counts.iter_mut().find(|c| c.0 == w) {
            Some(c) => c.1 += 1,
            None => counts.push((w, 1, i)),
        }
    }
    counts.retain(|c| c.1 >= 3 && c.0.chars().count() >= 4);
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    for (w, _, _) in counts {
        push(w, &mut tags);
    }
    tags
}

/// Tidy tags from anywhere (Claude, the user): trimmed, no `#`, no repeats (ignoring case),
/// at most [`MAX_TAGS`], and together within YouTube's 500 characters.
pub fn clean_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut total = 0;
    for t in tags {
        let t = t.trim().trim_start_matches('#').trim().to_string();
        if t.is_empty() || t.chars().count() > 60 || out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            continue;
        }
        total += t.chars().count() + 2;
        if out.len() >= MAX_TAGS || total > 500 {
            break;
        }
        out.push(t);
    }
    out
}

/// YouTube's longest title.
pub const MAX_TITLE: usize = 100;

/// A title within [`MAX_TITLE`] characters, on one line.
pub fn clean_title(t: &str) -> String {
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_matches(['"', '\u{201c}', '\u{201d}']).trim().to_string();
    if t.chars().count() <= MAX_TITLE {
        return t;
    }
    let mut cut: String = t.chars().take(MAX_TITLE - 1).collect();
    if let Some(i) = cut.rfind(' ') {
        cut.truncate(i);
    }
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Input {
        Input {
            name: "Deploying Rails to Fly".into(),
            transcript: "Hi everyone. Today we deploy a Rails app to Fly. First the Dockerfile, then the database. The database needs a volume. Then we check the database again and deploy.".into(),
            chapters: vec!["Intro".into(), "Dockerfile".into(), "Database".into()],
            chapter_list: "00:00 Intro\n00:20 Dockerfile\n01:05 Database".into(),
            duration_ms: 120_000.0,
        }
    }

    #[test]
    fn template_uses_the_name_opening_and_chapters() {
        let d = template(&input());
        assert_eq!(d.title, "Deploying Rails to Fly");
        assert!(d.description.starts_with("Hi everyone. Today we deploy"));
        assert!(d.description.ends_with("Chapters\n00:00 Intro\n00:20 Dockerfile\n01:05 Database"));
        assert!(!d.by_ai);
        // the name first, then chapter titles, then words said three times or more
        assert_eq!(d.tags, vec!["deploying", "rails", "fly", "intro", "dockerfile", "database"]);
    }

    #[test]
    fn template_without_transcript_or_chapters() {
        let d = template(&Input { name: "  ".into(), ..Default::default() });
        assert_eq!(d.title, "Untitled");
        assert_eq!(d.description, "");
        assert!(d.tags.is_empty());
        assert_eq!(d.to_text(), "Title\nUntitled\n\nDescription\n\n");
    }

    #[test]
    fn long_transcripts_open_with_whole_sentences() {
        let s = "This is a sentence of about fifty characters long. ".repeat(20);
        let o = opening(&s);
        assert!(o.ends_with('.'));
        assert!(o.chars().count() <= OPENING_CHARS);
        let one = "word ".repeat(200);
        let o = opening(&one);
        assert!(o.ends_with('…') && o.chars().count() <= OPENING_CHARS + 1);
    }

    #[test]
    fn tags_and_titles_are_tidied() {
        let tags = clean_tags(["#Rust".to_string(), "rust".into(), " ".into(), "screen recording".into()]);
        assert_eq!(tags, vec!["Rust", "screen recording"]);
        assert_eq!(clean_title("  \"A   title\"  "), "A title");
        let long = clean_title(&"word ".repeat(40));
        assert!(long.chars().count() <= MAX_TITLE && long.ends_with('…'));
    }

    #[test]
    fn text_file_lists_each_part() {
        let d = Details { title: "T".into(), description: "D".into(), tags: vec!["a".into(), "b".into()], by_ai: true };
        assert_eq!(d.to_text(), "Title\nT\n\nDescription\nD\n\nTags\na, b\n");
    }
}
