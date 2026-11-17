//! Claude writes an export's title, description and tags from its transcript (one request with
//! a JSON answer). The chapter list is added after, by [`sf_engine::details::with_chapters`],
//! so its times are the export's own and never made up.

use serde_json::{json, Value};
use sf_engine::details::{clean_tags, clean_title, with_chapters, Details, Input};

use crate::client::{send, Route};

/// Transcript characters sent at most (about 30 minutes of speech); the rest is left out and
/// Claude is told so.
const MAX_TRANSCRIPT: usize = 24_000;

const SYSTEM: &str = "You write the details people paste when they publish a screen recording on YouTube or similar sites: a title, a description and tags. \
Write from what is said in the video and its chapter titles; do not claim anything the transcript does not support. \
Title: specific and plain, at most 70 characters, no clickbait, no emoji, no quotes around it. \
Description: two to four short paragraphs in the speaker's language that say what the viewer will see and learn, first paragraph usable on its own as a summary. No hashtags, no links, no timestamps or chapter list (one is added after it). \
Tags: 5 to 12 short search phrases, lowercase unless a name needs capitals, most important first.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "description": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["title", "description", "tags"],
        "additionalProperties": false
    })
}

/// The request body for `input` (without the model, which [`send`] sets).
pub fn request(input: &Input) -> Value {
    let mut transcript: String = input.transcript.chars().take(MAX_TRANSCRIPT).collect();
    if transcript.len() < input.transcript.len() {
        transcript.push_str("\n[the rest of the transcript is left out]");
    }
    let chapters = if input.chapters.is_empty() { "(none)".to_string() } else { input.chapters.join("\n") };
    let transcript = if transcript.trim().is_empty() { "(no transcript: nothing is said, or it was not transcribed)".to_string() } else { transcript };
    let user = format!("Project name: {}\nLength: {} s\n\nChapters:\n{}\n\nTranscript:\n{}", input.name.trim(), (input.duration_ms / 1000.0).round(), chapters, transcript);
    json!({
        "max_tokens": 4000,
        "system": SYSTEM,
        "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": schema() } },
        "messages": [{ "role": "user", "content": user }]
    })
}

/// Read Claude's answer into details (with the chapter list under the description).
pub fn parse(resp: &Value, input: &Input) -> Result<Details, String> {
    match resp["stop_reason"].as_str() {
        Some("refusal") => return Err("Claude declined to write details for this video.".into()),
        Some("max_tokens") => return Err("Claude's answer was cut off.".into()),
        _ => {}
    }
    let text: String = resp["content"].as_array().map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect()).unwrap_or_default();
    let v: Value = serde_json::from_str(text.trim()).map_err(|e| format!("Claude's answer could not be read: {e}"))?;
    let title = clean_title(v["title"].as_str().unwrap_or(""));
    if title.is_empty() {
        return Err("Claude's answer has no title.".into());
    }
    let description = v["description"].as_str().unwrap_or("").trim().to_string();
    let tags = clean_tags(v["tags"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default());
    Ok(Details { title, description: with_chapters(&description, &input.chapter_list), tags, by_ai: true })
}

/// Ask Claude (through `route`, with `model`) for the details of an export.
pub fn draft(route: &Route, model: &str, input: &Input) -> Result<Details, String> {
    let resp = send(route, model, request(input))?;
    parse(&resp, input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Input {
        Input { name: "Demo".into(), transcript: "Hello there.".into(), chapters: vec!["Intro".into()], chapter_list: "00:00 Intro\n00:30 Setup\n01:00 Done".into(), duration_ms: 90_000.0 }
    }

    #[test]
    fn request_asks_for_json_with_the_transcript() {
        let r = request(&input());
        assert_eq!(r["output_config"]["format"]["type"], "json_schema");
        let user = r["messages"][0]["content"].as_str().unwrap();
        assert!(user.contains("Project name: Demo") && user.contains("Length: 90 s") && user.contains("Hello there."));
        let long = Input { transcript: "x".repeat(MAX_TRANSCRIPT + 10), ..input() };
        assert!(request(&long)["messages"][0]["content"].as_str().unwrap().contains("left out"));
    }

    #[test]
    fn answer_gets_the_chapter_list_and_tidy_tags() {
        let resp = json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": "{\"title\":\" Set up a demo \",\"description\":\"How to set it up.\",\"tags\":[\"#demo\",\"Demo\",\"setup\"]}" }] });
        let d = parse(&resp, &input()).unwrap();
        assert_eq!(d.title, "Set up a demo");
        assert_eq!(d.description, "How to set it up.\n\nChapters\n00:00 Intro\n00:30 Setup\n01:00 Done");
        assert_eq!(d.tags, vec!["demo", "setup"]);
        assert!(d.by_ai);
    }

    #[test]
    fn refusals_and_garbage_are_errors() {
        assert!(parse(&json!({ "stop_reason": "refusal", "content": [] }), &input()).is_err());
        assert!(parse(&json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": "nope" }] }), &input()).is_err());
        assert!(parse(&json!({ "stop_reason": "end_turn", "content": [{ "type": "text", "text": "{\"title\":\"\",\"description\":\"\",\"tags\":[]}" }] }), &input()).is_err());
    }
}
