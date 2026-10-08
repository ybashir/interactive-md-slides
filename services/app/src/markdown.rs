use std::{collections::HashSet, sync::LazyLock};

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::models::{InteractionDefinition, InteractionOption, ParsedSlide};

static SLIDE_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)<!--\s*interdeck-slide:\s*([a-zA-Z0-9][a-zA-Z0-9_-]*)\s*-->").unwrap()
});
static INTERACTION_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s):::interact\{([^}]*)\}\s*(.*?)\s*:::").unwrap());
static CHART_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s):::chart\{([^}]*)\}\s*(.*?)\s*:::").unwrap());
static ECHARTS_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s):::echarts(?:\{([^}]*)\})?\s*(.*?)\s*:::").unwrap());
static INLINE_CHART: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^::chart\{([^}]*)\}\s*$").unwrap());
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([a-zA-Z][a-zA-Z0-9_-]*)=(?:"([^"]*)"|'([^']*)'|([^\s]+))"#).unwrap()
});
static OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*-\s*\[([a-zA-Z0-9][a-zA-Z0-9_-]*)\]\s+(.+?)\s*$").unwrap()
});
static OPTION_IMAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\s+\{image=(?:"([^"]+)"|'([^']+)'|([^\s}]+))\}\s*$"#).unwrap());
static SURVEY_QUESTION_CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+\{([^}\n]+)\}\s*$").unwrap());
static ELEMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\s*(?:[-*+]\s+)?(.+?)\s+\{([^}\n]*interact=(?:"[^"]+"|'[^']+'|[^\s}]+)[^}\n]*)\}\s*$"#).unwrap()
});
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#{1,3}\s+(.+?)\s*$").unwrap());
static HEADMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\A---\n.*?\n---\n").unwrap());
static HEADMATTER_BODY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\A---[\t ]*\r?\n(.*?)\r?\n---(?:[\t ]*\r?\n|\z)").unwrap());
static HEADMATTER_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^title:[^\r\n]*$").unwrap());
static HEADMATTER_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9_-]*$").unwrap());
static MARKER_WRAPPED_FRONTMATTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ms)^---[\t ]*\r?\n[\t ]*<!--\s*interdeck-slide:\s*([a-zA-Z0-9][a-zA-Z0-9_-]*)\s*-->[\t ]*\r?\n(.*?\r?\n)---[\t ]*\r?\n[\t ]*<!--\s*interdeck-slide:\s*([a-zA-Z0-9][a-zA-Z0-9_-]*)\s*-->",
    )
    .unwrap()
});
static CONSECUTIVE_SLIDE_MARKERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^[\t ]*<!--\s*interdeck-slide:\s*([a-zA-Z0-9][a-zA-Z0-9_-]*)\s*-->[\t ]*\r?\n(?:[\t ]*\r?\n)*[\t ]*<!--\s*interdeck-slide:\s*([a-zA-Z0-9][a-zA-Z0-9_-]*)\s*-->",
    )
    .unwrap()
});
static VALID_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]{1,63}$").unwrap());

#[derive(Clone, Debug, Serialize)]
pub struct ParsedDeck {
    pub slides: Vec<ParsedSlide>,
    pub interactions: Vec<InteractionDefinition>,
    pub warnings: Vec<String>,
}

pub fn ensure_slide_markers(markdown: &str) -> String {
    let normalized = markdown.replace("\r\n", "\n");
    if SLIDE_MARKER.is_match(&normalized) {
        return normalized;
    }

    let (prefix, body) = HEADMATTER
        .find(&normalized)
        .map(|headmatter| normalized.split_at(headmatter.end()))
        .unwrap_or(("", normalized.as_str()));
    let sections = body
        .split("\n---\n")
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>();
    if sections.is_empty() {
        return normalized;
    }

    let mut used = HashSet::new();
    let rendered = sections
        .into_iter()
        .enumerate()
        .map(|(index, section)| {
            let heading = first_heading(section).unwrap_or_else(|| format!("slide-{}", index + 1));
            let base = slugify(&heading, index + 1);
            let mut key = base.clone();
            let mut suffix = 2;
            while !used.insert(key.clone()) {
                key = format!("{base}-{suffix}");
                suffix += 1;
            }
            format!("<!-- interdeck-slide: {key} -->\n\n{section}")
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n");
    format!("{prefix}{rendered}\n")
}

/// Canonicalize the boundary immediately before every marked slide after the
/// first. Stable Interdeck markers give us an unambiguous slide intent even
/// when an AI edit glues `---` to content, omits the delimiter entirely, or
/// forgets the blank lines Slidev expects around a slide boundary.
pub fn normalize_marked_slide_boundaries(markdown: &str) -> (String, usize) {
    let normalized = markdown.replace("\r\n", "\n");
    let markers = SLIDE_MARKER.find_iter(&normalized).collect::<Vec<_>>();
    if markers.len() < 2 {
        return (normalized, 0);
    }

    let mut repaired = normalized.clone();
    let mut repairs = 0;
    for pair in markers.windows(2).rev() {
        let range = pair[0].end()..pair[1].start();
        let gap = &normalized[range.clone()];
        let canonical = canonical_slide_gap(gap);
        if canonical != gap {
            repaired.replace_range(range, &canonical);
            repairs += 1;
        }
    }
    let repaired_markers = SLIDE_MARKER
        .find_iter(&repaired)
        .map(|marker| marker.end())
        .collect::<Vec<_>>();
    for marker_end in repaired_markers.into_iter().rev() {
        let whitespace_end = whitespace_after_marker(&repaired, marker_end);
        let existing = &repaired[marker_end..whitespace_end];
        if existing != "\n\n" {
            repaired.replace_range(marker_end..whitespace_end, "\n\n");
            repairs += 1;
        }
    }
    (repaired, repairs)
}

fn whitespace_after_marker(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = start;
    while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
        cursor += 1;
    }
    if cursor >= bytes.len() || bytes[cursor] != b'\n' {
        return cursor;
    }
    cursor += 1;
    loop {
        let line_start = cursor;
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b'\n' {
            cursor += 1;
        } else {
            return line_start;
        }
    }
}

fn content_before_boundary(content: &str) -> String {
    let content = content.trim();
    if content.is_empty() {
        String::new()
    } else {
        format!("\n\n{content}")
    }
}

fn canonical_slide_gap(gap: &str) -> String {
    let delimiter_positions = gap
        .match_indices("---")
        .filter_map(|(index, _)| {
            let bytes = gap.as_bytes();
            let before_is_hyphen = index > 0 && bytes[index - 1] == b'-';
            let after = index + 3;
            let after_is_hyphen = after < bytes.len() && bytes[after] == b'-';
            (!before_is_hyphen && !after_is_hyphen).then_some(index)
        })
        .collect::<Vec<_>>();

    let Some(&last) = delimiter_positions.last() else {
        return format!("{}\n\n---\n\n", content_before_boundary(gap));
    };
    // A thematic rule inside the preceding slide is content, not a boundary.
    // Only reuse the final delimiter when nothing except whitespace follows it
    // before the next stable marker.
    if !gap[last + 3..].trim().is_empty() {
        return format!("{}\n\n---\n\n", content_before_boundary(gap));
    }
    let frontmatter = delimiter_positions
        .iter()
        .rev()
        .skip(1)
        .find_map(|&opening| {
            let body = gap[opening + 3..last].trim();
            if body.is_empty() {
                return None;
            }
            matches!(
                serde_yaml::from_str::<serde_yaml::Value>(body),
                Ok(serde_yaml::Value::Mapping(_))
            )
            .then_some((opening, body))
        });

    if let Some((opening, body)) = frontmatter {
        format!(
            "{}\n\n---\n{}\n---\n\n",
            content_before_boundary(&gap[..opening]),
            body
        )
    } else {
        format!("{}\n\n---\n\n", content_before_boundary(&gap[..last]))
    }
}

pub fn repair_duplicate_slide_markers(markdown: &str) -> (String, Vec<(String, String)>) {
    let mut reserved = SLIDE_MARKER
        .captures_iter(markdown)
        .map(|captures| captures[1].to_owned())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut repairs = Vec::new();
    let repaired = SLIDE_MARKER.replace_all(markdown, |captures: &regex::Captures<'_>| {
        let original = &captures[1];
        if seen.insert(original.to_owned()) {
            return captures[0].to_owned();
        }

        let mut suffix = 2;
        let replacement = loop {
            let suffix_text = format!("-{suffix}");
            let stem_length = original
                .len()
                .min(64_usize.saturating_sub(suffix_text.len()));
            let candidate = format!("{}{}", &original[..stem_length], suffix_text);
            if !reserved.contains(&candidate) {
                reserved.insert(candidate.clone());
                seen.insert(candidate.clone());
                break candidate;
            }
            suffix += 1;
        };
        repairs.push((original.to_owned(), replacement.clone()));
        format!("<!-- interdeck-slide: {replacement} -->")
    });
    (repaired.into_owned(), repairs)
}

pub fn repair_redundant_slide_markers(markdown: &str) -> (String, usize) {
    let mut repairs = 0;
    // A normal slide before a frontmatter-bearing slide can form a larger,
    // non-repairable regex match that contains the smaller repairable match.
    // Regex::replace_all only visits non-overlapping matches, so discover the
    // equal-ID matches with an overlapping scan and apply them back-to-front.
    let mut frontmatter_repaired = markdown.to_owned();
    loop {
        let mut offset = 0;
        let mut repair = None;
        while offset < frontmatter_repaired.len() {
            let Some(captures) =
                MARKER_WRAPPED_FRONTMATTER.captures_at(frontmatter_repaired.as_str(), offset)
            else {
                break;
            };
            let Some(full_match) = captures.get(0) else {
                break;
            };
            if captures[1] == captures[3] {
                repair = Some((
                    full_match.range(),
                    format!(
                        "---\n{}---\n<!-- interdeck-slide: {} -->",
                        &captures[2], &captures[1]
                    ),
                ));
                break;
            }
            offset = full_match.start() + 1;
        }
        let Some((range, replacement)) = repair else {
            break;
        };
        frontmatter_repaired.replace_range(range, &replacement);
        repairs += 1;
    }
    let repaired = CONSECUTIVE_SLIDE_MARKERS.replace_all(
        frontmatter_repaired.as_str(),
        |captures: &regex::Captures<'_>| {
            if captures[1] != captures[2] {
                return captures[0].to_owned();
            }
            repairs += 1;
            format!("<!-- interdeck-slide: {} -->", &captures[1])
        },
    );
    (repaired.into_owned(), repairs)
}

pub fn repair_unquoted_headmatter_title(markdown: &str) -> (String, bool) {
    let Some(headmatter) = HEADMATTER_BODY.captures(markdown) else {
        return (markdown.to_owned(), false);
    };
    let Some(body) = headmatter.get(1) else {
        return (markdown.to_owned(), false);
    };
    if validate_deck_headmatter(markdown).is_ok() {
        return (markdown.to_owned(), false);
    }

    let repaired_body =
        HEADMATTER_TITLE.replace(body.as_str(), |captures: &regex::Captures<'_>| {
            let line = captures
                .get(0)
                .map(|value| value.as_str())
                .unwrap_or_default();
            let raw = line.strip_prefix("title:").unwrap_or_default().trim();
            let (value, comment) = raw
                .split_once(" #")
                .map(|(value, comment)| (value.trim_end(), format!(" #{comment}")))
                .unwrap_or((raw, String::new()));
            let decoded =
                serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned());
            let cleaned = decoded.replace("\\\"", "\"").replace('"', "");
            let quoted = serde_json::to_string(cleaned.trim())
                .unwrap_or_else(|_| format!("\"{}\"", cleaned.trim()));
            format!("title: {quoted}{comment}")
        });
    if repaired_body == body.as_str() {
        return (markdown.to_owned(), false);
    }

    let mut repaired = markdown.to_owned();
    repaired.replace_range(body.range(), &repaired_body);
    if validate_deck_headmatter(&repaired).is_err() {
        return (markdown.to_owned(), false);
    }
    (repaired, true)
}

pub fn validate_deck_headmatter(markdown: &str) -> Result<(), String> {
    let trimmed = markdown.trim_start_matches('\u{feff}');
    if !trimmed.starts_with("---") {
        return Ok(());
    }
    let captures = HEADMATTER_BODY
        .captures(trimmed)
        .ok_or_else(|| "The deck headmatter is missing its closing `---` marker".to_owned())?;
    let body = captures
        .get(1)
        .map(|value| value.as_str())
        .unwrap_or_default();
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(body)
        .map_err(|_| "The deck headmatter is not valid YAML".to_owned())?;
    let serde_yaml::Value::Mapping(mapping) = parsed else {
        if matches!(parsed, serde_yaml::Value::Null) {
            return Ok(());
        }
        return Err("The deck headmatter must be a YAML object".to_owned());
    };
    if mapping.keys().any(
        |key| !matches!(key, serde_yaml::Value::String(value) if HEADMATTER_KEY.is_match(value)),
    ) {
        return Err("The deck headmatter contains a malformed property name".to_owned());
    }
    Ok(())
}

pub fn parse_deck(markdown: &str) -> Result<ParsedDeck, String> {
    let normalized = markdown.replace("\r\n", "\n");
    let mut sections = sections_from_markers(&normalized);
    let mut warnings = Vec::new();

    if sections.is_empty() {
        warnings.push("Slides do not have stable Interdeck markers yet; add `<!-- interdeck-slide: a-stable-id -->` to every slide before relying on persistent interactions.".to_owned());
        sections = fallback_sections(&normalized);
    }

    if sections.is_empty() {
        return Err("The deck does not contain any slides".to_owned());
    }

    let mut slides = Vec::with_capacity(sections.len());
    let mut interactions = Vec::new();
    let mut chart_sources = Vec::new();
    let mut seen_slide_ids = HashSet::new();
    let mut seen_interaction_ids = HashSet::new();

    for (index, (key, source)) in sections.into_iter().enumerate() {
        if !seen_slide_ids.insert(key.clone()) {
            return Err(format!("Duplicate slide ID `{key}`"));
        }

        let number = (index + 1) as i32;
        let title = first_heading(source).unwrap_or_else(|| format!("Slide {number}"));
        slides.push(ParsedSlide {
            key: key.clone(),
            number,
            title: title.clone(),
        });

        for captures in CHART_BLOCK.captures_iter(source) {
            let attrs = parse_attributes(captures.get(1).map(|v| v.as_str()).unwrap_or_default());
            let body = captures.get(2).map(|v| v.as_str()).unwrap_or_default();
            validate_chart(&attrs, body)?;
            if let Some(source) = attrs.get("source").and_then(Value::as_str) {
                chart_sources.push(source.to_owned());
            }
        }
        for captures in ECHARTS_BLOCK.captures_iter(source) {
            let attrs = parse_attributes(captures.get(1).map(|v| v.as_str()).unwrap_or_default());
            let body = captures.get(2).map(|v| v.as_str()).unwrap_or_default();
            validate_echarts(&attrs, body)?;
            if let Some(source) = attrs.get("source").and_then(Value::as_str) {
                chart_sources.push(source.to_owned());
            }
        }
        for captures in INLINE_CHART.captures_iter(source) {
            let attrs = parse_attributes(captures.get(1).map(|v| v.as_str()).unwrap_or_default());
            validate_chart(&attrs, "")?;
            if let Some(source) = attrs.get("source").and_then(Value::as_str) {
                chart_sources.push(source.to_owned());
            }
        }

        for captures in INTERACTION_BLOCK.captures_iter(source) {
            let attrs = parse_attributes(captures.get(1).map(|v| v.as_str()).unwrap_or_default());
            let body = captures.get(2).map(|v| v.as_str()).unwrap_or_default();
            let id = required_attr(&attrs, "id")?;
            validate_id(&id, "interaction")?;
            if !seen_interaction_ids.insert(id.clone()) {
                return Err(format!("Duplicate interaction ID `{id}`"));
            }
            let kind = required_attr(&attrs, "type")?;
            validate_kind(&kind)?;
            let question = attrs
                .get("question")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| first_heading(body))
                .unwrap_or_else(|| title.clone());
            let (options, survey_questions) = if kind == "survey" {
                let (options, questions) = parse_survey_questions(&id, body)?;
                (options, Some(questions))
            } else {
                let options = OPTION
                    .captures_iter(body)
                    .map(|option| {
                        let raw_label = option[2].trim();
                        let image = OPTION_IMAGE.captures(raw_label);
                        let image_url = image.as_ref().and_then(|capture| {
                            capture
                                .get(1)
                                .or_else(|| capture.get(2))
                                .or_else(|| capture.get(3))
                                .map(|value| value.as_str().to_owned())
                        });
                        let label = image
                            .and_then(|capture| capture.get(0))
                            .map(|suffix| raw_label[..suffix.start()].trim())
                            .unwrap_or(raw_label)
                            .to_owned();
                        InteractionOption {
                            id: option[1].to_owned(),
                            label,
                            image_url,
                        }
                    })
                    .collect::<Vec<_>>();
                (options, None)
            };
            validate_options(&id, &kind, &options)?;
            validate_config(&id, &kind, &attrs, &options)?;

            let mut config = config_value(&attrs);
            if let (Some(questions), Some(config)) = (survey_questions, config.as_object_mut()) {
                config.insert("questions".to_owned(), Value::Array(questions));
            }

            interactions.push(InteractionDefinition {
                id,
                slide_key: key.clone(),
                slide_number: number,
                kind,
                title: question,
                options,
                config,
                element: false,
            });
        }

        for captures in ELEMENT.captures_iter(source) {
            let label = captures
                .get(1)
                .map(|v| clean_inline(v.as_str()))
                .unwrap_or_default();
            let attrs = parse_attributes(captures.get(2).map(|v| v.as_str()).unwrap_or_default());
            let id = required_attr(&attrs, "id")?;
            validate_id(&id, "interaction")?;
            if !seen_interaction_ids.insert(id.clone()) {
                return Err(format!("Duplicate interaction ID `{id}`"));
            }
            let kind = required_attr(&attrs, "interact")?;
            validate_kind(&kind)?;
            if !matches!(kind.as_str(), "vote" | "updown" | "reaction" | "rating") {
                return Err(format!(
                    "Element interaction `{id}` cannot use type `{kind}`; use updown, reaction, or rating"
                ));
            }
            let options = if kind == "reaction" {
                parse_element_reaction_options(&id, &attrs)?
            } else {
                Vec::new()
            };
            validate_options(&id, &kind, &options)?;
            validate_config(&id, &kind, &attrs, &options)?;
            let mut config = config_value(&attrs);
            if let Some(config) = config.as_object_mut() {
                config.remove("options");
            }

            interactions.push(InteractionDefinition {
                id,
                slide_key: key.clone(),
                slide_number: number,
                kind,
                title: label,
                options,
                config,
                element: true,
            });
        }
    }

    for source in chart_sources {
        if !seen_interaction_ids.contains(&source) {
            return Err(format!(
                "Chart source `{source}` does not match an interaction in this deck"
            ));
        }
    }

    Ok(ParsedDeck {
        slides,
        interactions,
        warnings,
    })
}

fn sections_from_markers(markdown: &str) -> Vec<(String, &str)> {
    let markers = SLIDE_MARKER.captures_iter(markdown).collect::<Vec<_>>();
    markers
        .iter()
        .enumerate()
        .map(|(index, capture)| {
            let start = capture.get(0).unwrap().start();
            let end = markers
                .get(index + 1)
                .and_then(|next| next.get(0))
                .map(|next| next.start())
                .unwrap_or(markdown.len());
            (capture[1].to_owned(), &markdown[start..end])
        })
        .collect()
}

fn fallback_sections(markdown: &str) -> Vec<(String, &str)> {
    let body = strip_initial_headmatter(markdown);
    body.split("\n---\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .enumerate()
        .map(|(index, part)| (format!("slide-{}", index + 1), part))
        .collect()
}

fn strip_initial_headmatter(markdown: &str) -> &str {
    if let Some(rest) = markdown.strip_prefix("---\n")
        && let Some(end) = rest.find("\n---\n")
    {
        return &rest[end + 5..];
    }
    markdown
}

fn parse_attributes(source: &str) -> Map<String, Value> {
    ATTRIBUTE
        .captures_iter(source)
        .filter_map(|capture| {
            let value = capture
                .get(2)
                .or_else(|| capture.get(3))
                .or_else(|| capture.get(4))?;
            Some((
                capture[1].to_owned(),
                Value::String(value.as_str().to_owned()),
            ))
        })
        .collect()
}

fn config_value(attrs: &Map<String, Value>) -> Value {
    let mut config = attrs.clone();
    config.remove("id");
    config.remove("type");
    config.remove("interact");
    config.remove("question");
    Value::Object(config)
}

fn required_attr(attrs: &Map<String, Value>, key: &str) -> Result<String, String> {
    attrs
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("Interaction is missing required `{key}` attribute"))
}

fn parse_element_reaction_options(
    interaction_id: &str,
    attrs: &Map<String, Value>,
) -> Result<Vec<InteractionOption>, String> {
    let source = required_attr(attrs, "options").map_err(|_| {
        format!("Element reaction `{interaction_id}` needs `options=\"like:👍|celebrate:🎉\"`")
    })?;
    let options = source
        .split('|')
        .map(|entry| {
            let (id, label) = entry.split_once(':').ok_or_else(|| {
                format!(
                    "Element reaction `{interaction_id}` options must use stable `id:Emoji` pairs"
                )
            })?;
            let id = id.trim();
            let label = label.trim();
            if id.is_empty() || label.is_empty() {
                return Err(format!(
                    "Element reaction `{interaction_id}` options must use stable `id:Emoji` pairs"
                ));
            }
            Ok(InteractionOption {
                id: id.to_owned(),
                label: label.to_owned(),
                image_url: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if options.len() > 8 {
        return Err(format!(
            "Element reaction `{interaction_id}` supports at most 8 options"
        ));
    }
    Ok(options)
}

fn validate_id(id: &str, subject: &str) -> Result<(), String> {
    if VALID_ID.is_match(id) {
        Ok(())
    } else {
        Err(format!(
            "Invalid {subject} ID `{id}`; use 2–64 letters, digits, underscores, or hyphens"
        ))
    }
}

fn validate_kind(kind: &str) -> Result<(), String> {
    const SUPPORTED: &[&str] = &[
        "poll",
        "quiz",
        "rating",
        "word-cloud",
        "free-text",
        "vote",
        "updown",
        "reaction",
        "ranked-list",
        "image-choice",
        "number",
        "allocation",
        "matrix",
        "ranking",
        "image-hotspot",
        "survey",
    ];
    if SUPPORTED.contains(&kind) {
        Ok(())
    } else {
        Err(format!("Unsupported interaction type `{kind}`"))
    }
}

fn validate_chart(attrs: &Map<String, Value>, body: &str) -> Result<(), String> {
    const SUPPORTED_ATTRIBUTES: &[&str] = &[
        "type",
        "title",
        "height",
        "legend",
        "legend-gap",
        "stacked",
        "values",
        "x-label",
        "y-label",
        "x-labels",
        "y-labels",
        "x-ticks",
        "y-ticks",
        "grid",
        "smooth",
        "colors",
        "source",
    ];
    if let Some(key) = attrs
        .keys()
        .find(|key| !SUPPORTED_ATTRIBUTES.contains(&key.as_str()))
    {
        return Err(format!("Unsupported chart attribute `{key}`"));
    }

    let kind = attrs
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("bar")
        .to_ascii_lowercase();
    if !matches!(
        kind.as_str(),
        "bar" | "line" | "area" | "pie" | "donut" | "scatter" | "radar" | "heatmap"
    ) {
        return Err(format!("Unsupported chart type `{kind}`"));
    }

    if let Some(height) = attrs.get("height").and_then(Value::as_str) {
        let height = height
            .parse::<i32>()
            .map_err(|_| "Chart `height` must be a number between 180 and 520".to_owned())?;
        if !(180..=520).contains(&height) {
            return Err("Chart `height` must be between 180 and 520".to_owned());
        }
    }
    if let Some(value) = attrs.get("legend").and_then(Value::as_str)
        && !matches!(value, "true" | "false" | "top" | "bottom")
    {
        return Err("Chart `legend` must be `top`, `bottom`, or `false`".to_owned());
    }
    for key in [
        "stacked", "values", "x-labels", "y-labels", "x-ticks", "y-ticks", "grid", "smooth",
    ] {
        if let Some(value) = attrs.get(key).and_then(Value::as_str)
            && !matches!(value, "true" | "false")
        {
            return Err(format!("Chart `{key}` must be `true` or `false`"));
        }
    }
    if let Some(gap) = attrs.get("legend-gap").and_then(Value::as_str) {
        let gap = gap
            .parse::<i32>()
            .map_err(|_| "Chart `legend-gap` must be a number between 0 and 80".to_owned())?;
        if !(0..=80).contains(&gap) {
            return Err("Chart `legend-gap` must be between 0 and 80".to_owned());
        }
    }
    for key in ["x-label", "y-label"] {
        if attrs
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.chars().count() > 80)
        {
            return Err(format!("Chart `{key}` must be at most 80 characters"));
        }
    }
    if let Some(value) = attrs.get("colors").and_then(Value::as_str) {
        let colors = value
            .split('|')
            .map(str::trim)
            .filter(|color| !color.is_empty())
            .collect::<Vec<_>>();
        if colors.len() > 8 || colors.iter().any(|color| !valid_chart_color(color)) {
            return Err(
                "Chart `colors` must contain at most eight pipe-separated hex colors".to_owned(),
            );
        }
    }

    if let Some(source) = attrs.get("source").and_then(Value::as_str) {
        validate_id(source, "chart source")?;
        return Ok(());
    }

    let lines = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 3 {
        return Err(
            "Chart data must be a Markdown table with a header and at least one data row"
                .to_owned(),
        );
    }
    let headers = split_chart_row(lines[0]);
    let separators = split_chart_row(lines[1]);
    if headers.len() < 2
        || separators.len() != headers.len()
        || !separators.iter().all(|cell| valid_chart_separator(cell))
    {
        return Err("Chart data needs a valid Markdown table separator row".to_owned());
    }
    if matches!(kind.as_str(), "scatter" | "heatmap") && headers.len() != 3 {
        return Err(format!(
            "{} charts need exactly three columns",
            if kind == "scatter" {
                "Scatter"
            } else {
                "Heatmap"
            }
        ));
    }
    if matches!(kind.as_str(), "pie" | "donut") && headers.len() != 2 {
        return Err("Pie and donut charts support exactly one numeric series".to_owned());
    }

    for (index, line) in lines.iter().skip(2).enumerate() {
        let cells = split_chart_row(line);
        let row = index + 3;
        if cells.len() != headers.len() {
            return Err(format!(
                "Chart table row {row} has {} columns; expected {}",
                cells.len(),
                headers.len()
            ));
        }
        if cells[0].is_empty() {
            return Err(format!("Chart table row {row} needs a label"));
        }
        let numeric_start = if kind == "heatmap" { 2 } else { 1 };
        for (column, cell) in cells.iter().enumerate().skip(numeric_start) {
            let normalized = cell.replace(',', "").trim_end_matches('%').to_owned();
            if normalized.parse::<f64>().is_err() {
                return Err(format!(
                    "Chart table row {row}, column {} must be numeric",
                    column + 1
                ));
            }
        }
    }
    Ok(())
}

fn validate_echarts(attrs: &Map<String, Value>, body: &str) -> Result<(), String> {
    const SUPPORTED_ATTRIBUTES: &[&str] = &["height", "source", "reveal"];
    if let Some(key) = attrs
        .keys()
        .find(|key| !SUPPORTED_ATTRIBUTES.contains(&key.as_str()))
    {
        return Err(format!("Unsupported ECharts wrapper attribute `{key}`"));
    }
    if let Some(height) = attrs.get("height").and_then(Value::as_str) {
        let height = height
            .parse::<i32>()
            .map_err(|_| "ECharts `height` must be a number between 180 and 520".to_owned())?;
        if !(180..=520).contains(&height) {
            return Err("ECharts `height` must be between 180 and 520".to_owned());
        }
    }
    if let Some(source) = attrs.get("source").and_then(Value::as_str) {
        validate_id(source, "ECharts source")?;
    }
    if let Some(reveal) = attrs.get("reveal").and_then(Value::as_str)
        && !matches!(reveal, "all" | "series")
    {
        return Err("ECharts `reveal` must be `all` or `series`".to_owned());
    }
    if body.len() > 100_000 {
        return Err("ECharts JSON must be at most 100 KB".to_owned());
    }
    let option = serde_json::from_str::<Value>(body.trim())
        .map_err(|error| format!("ECharts option is not valid JSON: {error}"))?;
    if !option.is_object() {
        return Err("ECharts option must be a JSON object".to_owned());
    }
    let mut nodes = 0usize;
    validate_echarts_value(&option, 0, &mut nodes)?;
    if attrs.get("source").and_then(Value::as_str).is_some() {
        validate_live_echarts_option(&option)?;
    }
    Ok(())
}

fn validate_echarts_value(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), String> {
    *nodes += 1;
    if *nodes > 20_000 {
        return Err("ECharts option is too complex".to_owned());
    }
    if depth > 20 {
        return Err("ECharts option nesting is too deep".to_owned());
    }
    match value {
        Value::String(value) => {
            if value.chars().count() > 10_000 {
                return Err("ECharts option contains an excessively long string".to_owned());
            }
            let normalized = value.trim().to_ascii_lowercase();
            let private_asset = normalized.starts_with("image:///api/decks/");
            if normalized.starts_with("http:")
                || normalized.starts_with("https:")
                || normalized.starts_with("data:")
                || normalized.starts_with("javascript:")
                || normalized.starts_with("file:")
                || normalized.starts_with("//")
                || (normalized.starts_with("image://") && !private_asset)
            {
                return Err("ECharts options cannot load external resources".to_owned());
            }
        }
        Value::Array(values) => {
            if values.len() > 5_000 {
                return Err("ECharts option arrays may contain at most 5,000 items".to_owned());
            }
            for value in values {
                validate_echarts_value(value, depth + 1, nodes)?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(key.as_str(), "__proto__" | "prototype" | "constructor") {
                    return Err(format!("ECharts option key `{key}` is not allowed"));
                }
                validate_echarts_value(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_live_echarts_option(option: &Value) -> Result<(), String> {
    let series = option
        .get("series")
        .and_then(Value::as_array)
        .filter(|series| !series.is_empty())
        .ok_or_else(|| "A live ECharts option needs at least one `series` entry".to_owned())?;
    if series.iter().any(|series| {
        series
            .as_object()
            .is_some_and(|series| series.contains_key("data"))
    }) {
        return Err(
            "A live ECharts series must omit `data`; Interdeck supplies it through the dataset"
                .to_owned(),
        );
    }
    if option.get("dataset").is_some_and(Value::is_array) {
        return Err("A live ECharts option supports one injected dataset object".to_owned());
    }
    if option
        .get("dataset")
        .and_then(Value::as_object)
        .is_some_and(|dataset| dataset.contains_key("source") || dataset.contains_key("transform"))
    {
        return Err(
            "A live ECharts dataset must omit `source` and `transform`; Interdeck supplies its source"
                .to_owned(),
        );
    }
    Ok(())
}

fn split_chart_row(line: &str) -> Vec<String> {
    let source = line.trim().trim_start_matches('|').trim_end_matches('|');
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in source.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '|' {
            cells.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(character);
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

fn valid_chart_separator(cell: &str) -> bool {
    let trimmed = cell.trim().trim_start_matches(':').trim_end_matches(':');
    trimmed.len() >= 3 && trimmed.chars().all(|character| character == '-')
}

fn valid_chart_color(color: &str) -> bool {
    let Some(hex) = color.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|character| character.is_ascii_hexdigit())
}

fn validate_options(id: &str, kind: &str, options: &[InteractionOption]) -> Result<(), String> {
    if matches!(
        kind,
        "poll" | "quiz" | "reaction" | "ranked-list" | "image-choice" | "allocation" | "ranking"
    ) && options.len() < 2
    {
        return Err(format!("Interaction `{id}` needs at least two options"));
    }
    if kind == "ranking" && options.len() > 20 {
        return Err(format!(
            "Ranking `{id}` supports at most 20 options for an accessible audience experience"
        ));
    }
    let mut seen = HashSet::new();
    for option in options {
        validate_id(&option.id, "option")?;
        if !seen.insert(&option.id) {
            return Err(format!(
                "Interaction `{id}` has duplicate option ID `{}`",
                option.id
            ));
        }
    }
    if kind == "image-choice" {
        for option in options {
            let image_url = option
                .image_url
                .as_deref()
                .ok_or_else(|| format!("Image choice `{id}` needs an image on every option"))?;
            if !is_safe_media_url(image_url) {
                return Err(format!(
                    "Image choice `{id}` uses an unsafe image URL; use HTTPS or an uploaded deck asset"
                ));
            }
        }
    }
    Ok(())
}

fn validate_config(
    id: &str,
    kind: &str,
    attrs: &Map<String, Value>,
    options: &[InteractionOption],
) -> Result<(), String> {
    if let Some(show_title) = attrs.get("show-title").and_then(Value::as_str)
        && !matches!(show_title, "true" | "false")
    {
        return Err(format!(
            "Interaction `{id}` has invalid `show-title`; use true or false"
        ));
    }
    if let Some(results) = attrs.get("results").and_then(Value::as_str)
        && !matches!(
            results,
            "after-vote" | "always" | "live" | "presenter" | "hidden" | "manual" | "on-close"
        )
    {
        return Err(format!(
            "Interaction `{id}` has unsupported result visibility `{results}`"
        ));
    }
    if let Some(timer) = attrs.get("timer").and_then(Value::as_str) {
        let seconds = timer
            .parse::<i32>()
            .map_err(|_| format!("Interaction `{id}` has invalid `timer`"))?;
        if !(5..=7200).contains(&seconds) {
            return Err(format!(
                "Interaction `{id}` needs `timer` between 5 and 7200 seconds"
            ));
        }
    }

    if kind == "poll" {
        if let Some(multiple) = attrs.get("multiple").and_then(Value::as_str)
            && !matches!(multiple, "true" | "false")
        {
            return Err(format!(
                "Interaction `{id}` has invalid `multiple`; use true or false"
            ));
        }
        if let Some(max) = attrs.get("max").and_then(Value::as_str) {
            let max = max
                .parse::<usize>()
                .map_err(|_| format!("Interaction `{id}` has invalid `max`"))?;
            if max == 0 || max > options.len() {
                return Err(format!(
                    "Interaction `{id}` needs `max` between 1 and {}",
                    options.len()
                ));
            }
        }
    }

    if kind == "word-cloud"
        && let Some(entries) = attrs.get("entries").and_then(Value::as_str)
        && !matches!(entries, "one" | "multiple")
    {
        return Err(format!(
            "Interaction `{id}` has invalid `entries`; use one or multiple"
        ));
    }

    if kind == "quiz"
        && let Some(correct) = attrs.get("correct").and_then(Value::as_str)
        && !options.iter().any(|option| option.id == correct)
    {
        return Err(format!(
            "Interaction `{id}` has unknown correct option `{correct}`"
        ));
    }

    if kind == "rating" {
        let min = numeric_attr(id, attrs, "min", 1)?;
        let max = numeric_attr(id, attrs, "max", 5)?;
        if min >= max || max - min > 10 {
            return Err(format!(
                "Interaction `{id}` needs an integer rating range of at most 10 values"
            ));
        }
    }
    if kind == "number" {
        let min = decimal_attr(id, attrs, "min", 0.0)?;
        let max = decimal_attr(id, attrs, "max", 100.0)?;
        let step = decimal_attr(id, attrs, "step", 1.0)?;
        if min >= max || step <= 0.0 || !min.is_finite() || !max.is_finite() || !step.is_finite() {
            return Err(format!(
                "Interaction `{id}` needs finite number bounds and a positive step"
            ));
        }
    }
    if kind == "allocation" {
        let total = numeric_attr(id, attrs, "total", 100)?;
        if !(1..=1000).contains(&total) {
            return Err(format!(
                "Interaction `{id}` needs `total` between 1 and 1000"
            ));
        }
    }
    if kind == "matrix" {
        let x_min = decimal_attr(id, attrs, "x-min", 0.0)?;
        let x_max = decimal_attr(id, attrs, "x-max", 10.0)?;
        let y_min = decimal_attr(id, attrs, "y-min", 0.0)?;
        let y_max = decimal_attr(id, attrs, "y-max", 10.0)?;
        if x_min >= x_max
            || y_min >= y_max
            || [x_min, x_max, y_min, y_max]
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(format!(
                "Interaction `{id}` needs valid finite matrix bounds"
            ));
        }
    }
    if kind == "image-hotspot" {
        let image = attrs
            .get("image")
            .and_then(Value::as_str)
            .filter(|value| is_safe_media_url(value))
            .ok_or_else(|| {
                format!("Image hotspot `{id}` needs a safe HTTPS or uploaded deck asset `image`")
            })?;
        if image.is_empty()
            || attrs
                .get("alt")
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!(
                "Image hotspot `{id}` needs descriptive `alt` text for accessibility"
            ));
        }
    }
    Ok(())
}

fn is_safe_media_url(value: &str) -> bool {
    value.starts_with("https://")
        || value.starts_with("/api/decks/")
        || ["ocean-hero.webp", "jellyfish.webp", "octopus.webp"]
            .iter()
            .any(|filename| value == format!("/samples/amazing-sea-creatures/{filename}"))
}

fn parse_survey_questions(
    interaction_id: &str,
    body: &str,
) -> Result<(Vec<InteractionOption>, Vec<Value>), String> {
    let captures = OPTION.captures_iter(body).collect::<Vec<_>>();
    if captures.is_empty() || captures.len() > 10 {
        return Err(format!(
            "Survey `{interaction_id}` needs between 1 and 10 questions"
        ));
    }
    let mut question_ids = HashSet::new();
    let mut questions = Vec::with_capacity(captures.len());
    let mut options = Vec::with_capacity(captures.len());
    for capture in captures {
        let question_id = capture[1].to_owned();
        validate_id(&question_id, "survey question")?;
        if !question_ids.insert(question_id.clone()) {
            return Err(format!(
                "Survey `{interaction_id}` has duplicate question ID `{question_id}`"
            ));
        }
        let raw = capture[2].trim();
        let question_config = SURVEY_QUESTION_CONFIG.captures(raw).ok_or_else(|| {
            format!(
                "Survey question `{question_id}` needs a configuration such as `{{type=\"rating\" min=\"1\" max=\"5\"}}`"
            )
        })?;
        let suffix = question_config.get(0).unwrap();
        let label = raw[..suffix.start()].trim();
        if label.is_empty() {
            return Err(format!(
                "Survey question `{question_id}` needs visible question text"
            ));
        }
        let attrs = parse_attributes(&question_config[1]);
        let kind = required_attr(&attrs, "type")?;
        let required = match attrs.get("required").and_then(Value::as_str) {
            None | Some("true") => true,
            Some("false") => false,
            Some(_) => {
                return Err(format!(
                    "Survey question `{question_id}` has invalid `required`; use true or false"
                ));
            }
        };
        let question = match kind.as_str() {
            "rating" => {
                let min = numeric_attr(&question_id, &attrs, "min", 1)?;
                let max = numeric_attr(&question_id, &attrs, "max", 5)?;
                if min >= max || max - min > 10 {
                    return Err(format!(
                        "Survey rating `{question_id}` needs an integer range of at most 10 values"
                    ));
                }
                json!({ "id": question_id, "label": label, "type": kind, "required": required, "min": min, "max": max })
            }
            "choice" => {
                let choice_source = required_attr(&attrs, "options")?;
                let mut choice_ids = HashSet::new();
                let choices = choice_source
                    .split('|')
                    .map(|choice| {
                        let (id, label) = choice.split_once(':').ok_or_else(|| {
                            format!(
                                "Survey choice `{question_id}` options must use `id:Label|id:Label`"
                            )
                        })?;
                        validate_id(id, "survey choice")?;
                        if label.trim().is_empty() || !choice_ids.insert(id.to_owned()) {
                            return Err(format!(
                                "Survey choice `{question_id}` has an invalid or duplicate option"
                            ));
                        }
                        Ok(json!({ "id": id, "label": label.trim() }))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if !(2..=10).contains(&choices.len()) {
                    return Err(format!(
                        "Survey choice `{question_id}` needs between 2 and 10 options"
                    ));
                }
                json!({ "id": question_id, "label": label, "type": kind, "required": required, "options": choices })
            }
            "text" => {
                let max = numeric_attr(&question_id, &attrs, "max", 500)?;
                if !(1..=1000).contains(&max) {
                    return Err(format!(
                        "Survey text `{question_id}` needs `max` between 1 and 1000"
                    ));
                }
                json!({ "id": question_id, "label": label, "type": kind, "required": required, "max": max })
            }
            _ => {
                return Err(format!(
                    "Survey question `{question_id}` has unsupported type `{kind}`; use rating, choice, or text"
                ));
            }
        };
        options.push(InteractionOption {
            id: question_id,
            label: label.to_owned(),
            image_url: None,
        });
        questions.push(question);
    }
    Ok((options, questions))
}

fn numeric_attr(
    id: &str,
    attrs: &Map<String, Value>,
    key: &str,
    default: i64,
) -> Result<i64, String> {
    attrs
        .get(key)
        .and_then(Value::as_str)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| format!("Interaction `{id}` has invalid `{key}`"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn decimal_attr(
    id: &str,
    attrs: &Map<String, Value>,
    key: &str,
    default: f64,
) -> Result<f64, String> {
    attrs
        .get(key)
        .and_then(Value::as_str)
        .map(|value| {
            value
                .parse::<f64>()
                .map_err(|_| format!("Interaction `{id}` has invalid `{key}`"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn first_heading(source: &str) -> Option<String> {
    HEADING
        .captures(source)
        .map(|capture| clean_inline(&capture[1]))
}

fn clean_inline(source: &str) -> String {
    source
        .replace("**", "")
        .replace(['`', '_'], "")
        .trim()
        .to_owned()
}

fn slugify(value: &str, index: usize) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            pending_dash = false;
        } else if !slug.is_empty() {
            pending_dash = true;
        }
    }
    if slug.len() < 2 {
        format!("slide-{index}")
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_slide_and_element_interactions_with_stable_ids() {
        let markdown = r#"---
theme: default
---
<!-- interdeck-slide: welcome -->
# Welcome

---
<!-- interdeck-slide: priorities -->
# Priorities

:::interact{type="poll" id="roadmap-priority" results="after-vote"}
# What should we prioritize?
- [reliability] Reliability
- [delivery-speed] Delivery speed
:::

- Ship weekly {interact="vote" id="ship-weekly"}
"#;

        let parsed = parse_deck(markdown).unwrap();
        assert_eq!(parsed.slides.len(), 2);
        assert_eq!(parsed.slides[1].key, "priorities");
        assert_eq!(parsed.interactions.len(), 2);
        assert_eq!(parsed.interactions[0].id, "roadmap-priority");
        assert_eq!(parsed.interactions[0].options.len(), 2);
        assert_eq!(parsed.interactions[1].id, "ship-weekly");
        assert!(parsed.interactions[1].element);
    }

    #[test]
    fn parses_inline_emoji_reactions_with_stable_options() {
        let markdown = r#"<!-- interdeck-slide: progress -->
# Progress

- Investment is paying off {interact="reaction" id="investment-reaction" options="like:👍|celebrate:🎉|question:🤔"}
"#;

        let parsed = parse_deck(markdown).unwrap();
        let interaction = &parsed.interactions[0];
        assert_eq!(interaction.kind, "reaction");
        assert!(interaction.element);
        assert_eq!(interaction.options.len(), 3);
        assert_eq!(interaction.options[0].id, "like");
        assert_eq!(interaction.options[0].label, "👍");
        assert!(interaction.config.get("options").is_none());
    }

    #[test]
    fn validates_inline_emoji_reaction_options() {
        let missing = r#"<!-- interdeck-slide: progress -->
# Progress
- Investment is paying off {interact="reaction" id="investment-reaction"}
"#;
        assert!(parse_deck(missing).unwrap_err().contains("needs `options="));

        let duplicate = r#"<!-- interdeck-slide: progress -->
# Progress
- Investment is paying off {interact="reaction" id="investment-reaction" options="like:👍|like:🎉"}
"#;
        assert!(
            parse_deck(duplicate)
                .unwrap_err()
                .contains("duplicate option ID")
        );
    }

    #[test]
    fn rejects_duplicate_interaction_ids() {
        let markdown = r#"<!-- interdeck-slide: one -->
# One
- First {interact="vote" id="same-id"}
---
<!-- interdeck-slide: two -->
# Two
- Second {interact="vote" id="same-id"}
"#;
        assert!(
            parse_deck(markdown)
                .unwrap_err()
                .contains("Duplicate interaction ID")
        );
    }

    #[test]
    fn parses_interaction_first_ranked_list() {
        let markdown = r#"<!-- interdeck-slide: priorities -->
# Priorities

:::interact{type="ranked-list" id="priorities-rank" display="slide" reveal="click" results="on-close"}
# Which priorities matter most?
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::
"#;

        let parsed = parse_deck(markdown).unwrap();
        let interaction = &parsed.interactions[0];
        assert_eq!(interaction.kind, "ranked-list");
        assert_eq!(interaction.options.len(), 3);
        assert_eq!(interaction.config["display"], "slide");
        assert_eq!(interaction.config["reveal"], "click");
    }

    #[test]
    fn validates_pilot_interaction_configuration() {
        let markdown = r#"<!-- interdeck-slide: check-in -->
# Check-in

:::interact{type="poll" id="multi-poll" multiple="true" max="2"}
# Pick two
- [one] One
- [two] Two
- [three] Three
:::

:::interact{type="quiz" id="quick-quiz" correct="two" results="manual" timer="30"}
# Choose the answer
- [one] One
- [two] Two
:::

:::interact{type="reaction" id="room-reaction"}
# React
- [love] ❤️ Love it
- [question] ❓ Question
:::
"#;

        let parsed = parse_deck(markdown).unwrap();
        assert_eq!(parsed.interactions.len(), 3);
        assert_eq!(parsed.interactions[0].config["max"], "2");
        assert_eq!(parsed.interactions[1].config["correct"], "two");
        assert_eq!(parsed.interactions[1].config["timer"], "30");
        assert_eq!(parsed.interactions[2].kind, "reaction");

        let word_cloud = r#"<!-- interdeck-slide: cloud -->
# Cloud
:::interact{type="word-cloud" id="cloud" entries="one"}
:::
"#;
        let parsed_word_cloud = parse_deck(word_cloud).unwrap();
        assert_eq!(parsed_word_cloud.interactions[0].config["entries"], "one");
        assert!(
            parse_deck(&word_cloud.replace("entries=\"one\"", "entries=\"many\""))
                .unwrap_err()
                .contains("use one or multiple")
        );

        let titled_poll = markdown.replace(
            "type=\"poll\" id=\"multi-poll\"",
            "type=\"poll\" id=\"multi-poll\" show-title=\"true\"",
        );
        let parsed_titled_poll = parse_deck(&titled_poll).unwrap();
        assert_eq!(
            parsed_titled_poll.interactions[0].config["show-title"],
            "true"
        );
        assert!(
            parse_deck(&titled_poll.replace("show-title=\"true\"", "show-title=\"yes\""))
                .unwrap_err()
                .contains("use true or false")
        );

        let invalid_quiz = markdown.replace("correct=\"two\"", "correct=\"missing\"");
        assert!(
            parse_deck(&invalid_quiz)
                .unwrap_err()
                .contains("unknown correct option")
        );

        let invalid_rating = r#"<!-- interdeck-slide: scale -->
# Scale
:::interact{type="rating" id="wide-scale" min="1" max="20"}
# Too wide
:::
"#;
        assert!(
            parse_deck(invalid_rating)
                .unwrap_err()
                .contains("rating range")
        );

        let invalid_timer = markdown.replace("timer=\"30\"", "timer=\"3\"");
        assert!(parse_deck(&invalid_timer).unwrap_err().contains("timer"));

        let invalid_visibility = markdown.replace("results=\"manual\"", "results=\"later\"");
        assert!(
            parse_deck(&invalid_visibility)
                .unwrap_err()
                .contains("result visibility")
        );
    }

    #[test]
    fn parses_structured_and_image_interactions() {
        let markdown = r#"<!-- interdeck-slide: structured -->
# Structured inputs

:::interact{type="image-choice" id="visual-choice"}
# Choose a direction
- [calm] Calm {image="https://example.com/calm.png"}
- [bold] Bold {image="/api/decks/00000000-0000-4000-8000-000000000001/assets/00000000-0000-4000-8000-000000000002/content"}
:::

:::interact{type="number" id="estimate" min="10" max="500" step="5" unit="days"}
# Estimate the duration
:::

:::interact{type="allocation" id="budget" total="100"}
# Allocate the budget
- [quality] Quality
- [speed] Speed
:::

:::interact{type="matrix" id="priority-matrix" x-min="0" x-max="10" y-min="-5" y-max="5"}
# Place the idea
:::
"#;
        let parsed = parse_deck(markdown).unwrap();
        assert_eq!(parsed.interactions.len(), 4);
        assert_eq!(parsed.interactions[0].options[0].label, "Calm");
        assert_eq!(
            parsed.interactions[0].options[0].image_url.as_deref(),
            Some("https://example.com/calm.png")
        );
        assert_eq!(parsed.interactions[1].config["unit"], "days");
        assert_eq!(parsed.interactions[2].config["total"], "100");

        let unsafe_image = markdown.replace("https://example.com/calm.png", "javascript:alert(1)");
        assert!(
            parse_deck(&unsafe_image)
                .unwrap_err()
                .contains("unsafe image")
        );
    }

    #[test]
    fn validates_ordering_and_accessible_image_hotspots() {
        let markdown = r#"<!-- interdeck-slide: parity -->
# Parity

:::interact{type="ranking" id="priorities"}
# Rank the priorities
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::

:::interact{type="image-hotspot" id="focus" image="https://example.com/map.png" alt="Map of the office floor"}
# Where should the collaboration space go?
:::
"#;
        let parsed = parse_deck(markdown).unwrap();
        assert_eq!(parsed.interactions.len(), 2);
        assert_eq!(parsed.interactions[0].kind, "ranking");
        assert_eq!(
            parsed.interactions[1].config["alt"],
            "Map of the office floor"
        );

        assert!(
            parse_deck(&markdown.replace(" alt=\"Map of the office floor\"", ""))
                .unwrap_err()
                .contains("descriptive `alt`")
        );
        assert!(
            parse_deck(&markdown.replace("https://example.com/map.png", "javascript:alert(1)"))
                .unwrap_err()
                .contains("safe HTTPS")
        );
    }

    #[test]
    fn parses_mixed_surveys_with_stable_question_and_choice_ids() {
        let markdown = r#"<!-- interdeck-slide: survey -->
# Pulse

:::interact{type="survey" id="team-pulse" results="presenter"}
# Quick pulse
- [confidence] How confident are you? {type="rating" min="1" max="5"}
- [priority] Which priority matters? {type="choice" options="quality:Quality|speed:Speed|learning:Learning"}
- [comment] What should improve? {type="text" max="500" required="false"}
:::
"#;
        let parsed = parse_deck(markdown).unwrap();
        let survey = &parsed.interactions[0];
        assert_eq!(survey.kind, "survey");
        assert_eq!(survey.options[0].label, "How confident are you?");
        assert_eq!(survey.config["questions"][0]["type"], "rating");
        assert_eq!(survey.config["questions"][1]["options"][1]["id"], "speed");
        assert_eq!(survey.config["questions"][2]["required"], false);

        assert!(
            parse_deck(&markdown.replace(
                "quality:Quality|speed:Speed|learning:Learning",
                "quality:Quality"
            ))
            .unwrap_err()
            .contains("between 2 and 10 options")
        );
        assert!(
            parse_deck(&markdown.replace("type=\"text\"", "type=\"email\""))
                .unwrap_err()
                .contains("unsupported type")
        );
    }

    #[test]
    fn adds_stable_unique_markers_to_imported_slidev_markdown() {
        let markdown = "---\ntheme: default\n---\n# Welcome\n\n---\n# Welcome\n";
        let normalized = ensure_slide_markers(markdown);

        assert!(normalized.contains("<!-- interdeck-slide: welcome -->"));
        assert!(normalized.contains("<!-- interdeck-slide: welcome-2 -->"));
        assert_eq!(parse_deck(&normalized).unwrap().slides.len(), 2);
        assert_eq!(ensure_slide_markers(&normalized), normalized);
    }

    #[test]
    fn normalizes_ai_generated_slide_boundaries_and_marker_spacing() {
        let malformed = r#"---
theme: default
---
<!-- interdeck-slide: first --># First---<!-- interdeck-slide: second --># Second
"#;
        let (repaired, count) = normalize_marked_slide_boundaries(malformed);

        assert_eq!(count, 2);
        assert!(repaired.contains(
            "<!-- interdeck-slide: first -->\n\n# First\n\n---\n\n<!-- interdeck-slide: second -->\n\n# Second"
        ));
        assert_eq!(normalize_marked_slide_boundaries(&repaired), (repaired, 0));
    }

    #[test]
    fn normalizes_glued_per_slide_frontmatter_without_losing_it() {
        let malformed = r#"<!-- interdeck-slide: first -->
# First---layout: center---<!-- interdeck-slide: second --># Second
"#;
        let (repaired, _) = normalize_marked_slide_boundaries(malformed);

        assert!(repaired.contains(
            "# First\n\n---\nlayout: center\n---\n\n<!-- interdeck-slide: second -->\n\n# Second"
        ));
    }

    #[test]
    fn inserts_a_missing_boundary_and_preserves_a_thematic_rule_as_content() {
        let malformed = r#"<!-- interdeck-slide: first -->
# First
---
Closing thought
<!-- interdeck-slide: second -->
# Second
"#;
        let (repaired, _) = normalize_marked_slide_boundaries(malformed);

        assert!(
            repaired.contains(
                "# First\n---\nClosing thought\n\n---\n\n<!-- interdeck-slide: second -->"
            )
        );
        assert_eq!(parse_deck(&repaired).unwrap().slides.len(), 2);
    }

    #[test]
    fn repairs_only_duplicate_slide_markers_without_colliding_with_existing_ids() {
        let markdown = r#"<!-- interdeck-slide: transition-bottlenecks -->
# First

---
<!-- interdeck-slide: transition-bottlenecks -->
# Second

---
<!-- interdeck-slide: transition-bottlenecks-2 -->
# Third
"#;
        let (repaired, repairs) = repair_duplicate_slide_markers(markdown);

        assert_eq!(
            repairs,
            vec![(
                "transition-bottlenecks".to_owned(),
                "transition-bottlenecks-3".to_owned()
            )]
        );
        assert_eq!(parse_deck(&repaired).unwrap().slides.len(), 3);
        assert!(repaired.contains("<!-- interdeck-slide: transition-bottlenecks-3 -->"));
        assert!(repaired.contains("<!-- interdeck-slide: transition-bottlenecks-2 -->"));
    }

    #[test]
    fn removes_redundant_markers_around_slide_frontmatter_and_content() {
        let markdown = r#"---
theme: default
---
<!-- interdeck-slide: intro -->
# Intro

---
<!-- interdeck-slide: detail -->
layout: two-cols
---
<!-- interdeck-slide: detail -->
# Detail

---
<!-- interdeck-slide: close -->

<!-- interdeck-slide: close -->
# Close
"#;
        let (repaired, count) = repair_redundant_slide_markers(markdown);

        assert_eq!(count, 2);
        assert_eq!(parse_deck(&repaired).unwrap().slides.len(), 3);
        assert!(repaired.contains("---\nlayout: two-cols\n---\n<!-- interdeck-slide: detail -->"));
    }

    #[test]
    fn quotes_an_unsafe_generated_headmatter_title_and_validates_it() {
        let markdown = "---\ntheme: default\ntitle: The Climate Challenge: Navigating Global Crisis & Action\n---\n<!-- interdeck-slide: intro -->\n# Climate\n";
        let (repaired, changed) = repair_unquoted_headmatter_title(markdown);

        assert!(changed);
        assert!(
            repaired
                .contains("title: \"The Climate Challenge: Navigating Global Crisis & Action\"")
        );
        assert_eq!(validate_deck_headmatter(&repaired), Ok(()));
    }

    #[test]
    fn rejects_invalid_initial_headmatter() {
        let markdown = "---\ntheme: [default\n---\n<!-- interdeck-slide: intro -->\n# Climate\n";
        assert_eq!(
            validate_deck_headmatter(markdown),
            Err("The deck headmatter is not valid YAML".to_owned())
        );
    }

    #[test]
    fn repairs_a_partially_quoted_generated_title_that_yaml_treats_as_a_key() {
        let markdown = "---\ntheme: default\ntitle:\"\\\"The Climate Challenge: Navigating Global\\\" Crisis & Action\"\n---\n<!-- interdeck-slide: intro -->\n# Climate\n";
        let (repaired, changed) = repair_unquoted_headmatter_title(markdown);

        assert!(changed);
        assert!(
            repaired
                .contains("title: \"The Climate Challenge: Navigating Global Crisis & Action\"")
        );
        assert_eq!(validate_deck_headmatter(&repaired), Ok(()));
    }
}
