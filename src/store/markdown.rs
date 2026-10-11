use std::{collections::HashSet, fmt::Write, ops::Range};

use anyhow::{Context, Result, bail, ensure};
use chrono::NaiveDate;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use serde::{Serialize, de::DeserializeOwned};

use crate::model::{Item, ItemKind};

pub struct Document<T> {
    pub metadata: T,
    yaml: String,
    pub body: String,
    newline: &'static str,
}

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

impl<T: DeserializeOwned + Serialize> Document<T> {
    pub fn parse(source: &str) -> Result<Self> {
        let newline = newline_of(source);
        let mut lines = source.split_inclusive('\n');
        ensure!(
            lines.next().map(str::trim) == Some("---"),
            "Missing YAML frontmatter"
        );
        let mut offset = source.find('\n').context("Incomplete frontmatter")? + 1;
        let start = offset;
        for line in lines {
            if line.trim() == "---" {
                let yaml = source[start..offset].to_owned();
                let metadata = serde_saphyr::from_str(&yaml).context("Invalid YAML frontmatter")?;
                return Ok(Self {
                    metadata,
                    yaml,
                    body: source[offset + line.len()..].to_owned(),
                    newline,
                });
            }
            offset += line.len();
        }
        bail!("Frontmatter has no closing delimiter")
    }

    pub fn new(metadata: T, body: String) -> Result<Self> {
        let yaml = serde_saphyr::to_string(&metadata)?;
        Ok(Self {
            metadata,
            yaml,
            body,
            newline: "\n",
        })
    }

    pub fn render(&self) -> String {
        format!("---{0}{1}---{0}{2}", self.newline, self.yaml, self.body)
    }

    /// Replaces one top-level frontmatter field, preserving the formatting of the others.
    pub fn set_field(&mut self, key: &str, value: impl Serialize) -> Result<()> {
        let value = serde_json::to_value(value)?;
        let mut expected: serde_json::Value = serde_saphyr::from_str(&self.yaml)?;
        let exists = expected
            .as_object_mut()
            .context("Frontmatter must be a mapping")?
            .insert(key.to_owned(), value.clone())
            .is_some();
        let range = self.field_range(key, exists)?;
        let replacement = self.field_text(key, &value, &range)?;
        let mut candidate = self.yaml.clone();
        candidate.replace_range(range, &replacement);
        let actual: serde_json::Value = serde_saphyr::from_str(&candidate)?;
        ensure!(
            actual == expected,
            "Editing '{key}' would affect other YAML fields; file preserved"
        );
        let metadata = serde_saphyr::from_str(&candidate)?;
        self.yaml = candidate;
        self.metadata = metadata;
        Ok(())
    }

    fn field_range(&self, key: &str, exists: bool) -> Result<Range<usize>> {
        let fields = top_level_fields(&self.yaml);
        if let Some(index) = fields.iter().position(|(field, _)| field == key) {
            let end = fields
                .get(index + 1)
                .map_or(self.yaml.len(), |(_, offset)| *offset);
            return Ok(fields[index].1..end);
        }
        ensure!(
            !exists,
            "Cannot edit '{key}' without reformatting YAML; use one key per line"
        );
        Ok(self.yaml.len()..self.yaml.len())
    }

    /// The new field line, keeping comments that belonged to the replaced text.
    fn field_text(
        &self,
        key: &str,
        value: &serde_json::Value,
        range: &Range<usize>,
    ) -> Result<String> {
        let newline = self.newline;
        let previous = &self.yaml[range.clone()];
        let mut text = format!("{key}: {}{newline}", serde_json::to_string(value)?);
        for line in previous
            .lines()
            .filter(|line| line.trim_start().starts_with('#'))
        {
            text.push_str(line);
            text.push_str(newline);
        }
        if let Some((_, comment)) = previous
            .lines()
            .next()
            .and_then(|line| line.split_once(" #"))
        {
            write!(text, "#{comment}{newline}")?;
        }
        Ok(text)
    }
}

/// Top-level mapping keys of a YAML document with their byte offsets.
fn top_level_fields(yaml: &str) -> Vec<(String, usize)> {
    let mut fields = Vec::new();
    let mut offset = 0;
    for line in yaml.split_inclusive('\n') {
        if !line.starts_with(char::is_whitespace)
            && !line.starts_with('#')
            && let Some((candidate, _)) = line.split_once(':')
            && let Ok(candidate) = serde_saphyr::from_str::<String>(candidate)
        {
            fields.push((candidate, offset));
        }
        offset += line.len();
    }
    fields
}

struct Section {
    title: String,
    content: Range<usize>,
}

fn sections(body: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut heading = None;
    let mut depth = 0usize;
    for (event, range) in Parser::new(body).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) if depth == 0 && level <= HeadingLevel::H2 => {
                if let Some(last) = sections.last_mut()
                    && last.content.end == body.len()
                {
                    last.content.end = range.start;
                }
                heading = Some((level, String::new(), range.end));
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, title, _)) = heading.as_mut() {
                    title.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                depth = depth.saturating_sub(1);
                if let Some((HeadingLevel::H2, title, end)) = heading.take() {
                    sections.push(Section {
                        title,
                        content: end..body.len(),
                    });
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    sections
}

fn section(body: &str, title: &str) -> Result<Option<Range<usize>>> {
    let matches: Vec<_> = sections(body)
        .into_iter()
        .filter(|section| section.title.eq_ignore_ascii_case(title))
        .collect();
    ensure!(
        matches.len() <= 1,
        "Duplicate '{title}' section; fix the Markdown before editing"
    );
    Ok(matches.into_iter().next().map(|section| section.content))
}

pub fn section_text(body: &str, title: &str) -> Result<String> {
    Ok(section(body, title)?.map_or_else(String::new, |range| body[range].trim().to_owned()))
}

pub fn set_section(body: &mut String, title: &str, text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty(),
        "The '{title}' content cannot be empty"
    );
    let newline = newline_of(body);
    let text = text.trim().replace("\r\n", "\n").replace('\n', newline);
    let mut candidate = body.clone();
    if let Some(range) = section(body, title)? {
        candidate.replace_range(range, &format!("{newline}{text}{newline}{newline}"));
    } else {
        write!(
            candidate,
            "{newline}## {title}{newline}{newline}{text}{newline}"
        )?;
    }
    ensure!(
        section_text(&candidate, title)? == text,
        "The '{title}' content cannot introduce another level 1 or 2 heading"
    );
    *body = candidate;
    Ok(())
}

pub struct LocatedItem {
    pub item: Item,
    pub range: Range<usize>,
}

fn located_items(body: &str, kind: ItemKind) -> Result<Vec<LocatedItem>> {
    let Some(content) = section(body, kind.section())? else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    let mut list_depth = 0usize;
    let mut quote_depth = 0usize;
    for (event, range) in Parser::new(&body[content.clone()]).into_offset_iter() {
        match event {
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth -= 1,
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote_depth -= 1,
            Event::Start(Tag::Item) if list_depth == 1 && quote_depth == 0 => {
                let range = content.start + range.start..content.start + range.end;
                if let Some(item) = parse_item(&body[range.clone()]) {
                    result.push(LocatedItem { item, range });
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

/// Items of a section, in document order.
pub fn items(body: &str, kind: ItemKind) -> Result<Vec<Item>> {
    Ok(located_items(body, kind)?
        .into_iter()
        .map(|entry| entry.item)
        .collect())
}

/// Items still open (not checked) in a section.
pub fn open_items(body: &str, kind: ItemKind) -> Result<Vec<Item>> {
    Ok(items(body, kind)?
        .into_iter()
        .filter(|item| !item.done)
        .collect())
}

fn parse_item(source: &str) -> Option<Item> {
    let source = source.trim();
    let marker_end = source.find(char::is_whitespace)?;
    let marker = &source[..marker_end];
    if !matches!(marker, "-" | "*" | "+") && !marker.ends_with('.') && !marker.ends_with(')') {
        return None;
    }
    let mut text = source[marker_end..].trim_start();
    let mut done = false;
    if let Some(rest) = text.strip_prefix("[ ]") {
        text = rest.trim_start();
    } else if let Some(rest) = text
        .strip_prefix("[x]")
        .or_else(|| text.strip_prefix("[X]"))
    {
        done = true;
        text = rest.trim_start();
    }
    let token = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(':');
    let id = ItemKind::ALL
        .iter()
        .any(|kind| kind.owns(token))
        .then(|| token.to_owned());
    Some(Item {
        id,
        done,
        text: text.to_owned(),
    })
}

pub fn add_item(body: &mut String, kind: ItemKind, text: &str, date: NaiveDate) -> Result<String> {
    single_line(text)?;
    let title = kind.section();
    let existing = located_items(body, kind)?;
    let mut max = 0u64;
    let mut ids = HashSet::new();
    for entry in &existing {
        if let Some(id) = &entry.item.id {
            ensure!(ids.insert(id), "Duplicate ID in '{title}': {id}");
            if let Some(number) = id.strip_prefix(kind.prefix()) {
                max = max.max(number.parse::<u64>().context("Item ID is out of range")?);
            }
        }
    }
    // Decisions without IDs remain readable and also occupy a sequence position.
    let unnumbered = existing
        .iter()
        .filter(|entry| entry.item.id.is_none())
        .count() as u64;
    max = max.max(unnumbered);
    let id = format!(
        "{}{}",
        kind.prefix(),
        max.checked_add(1).context("Item ID limit reached")?
    );
    let checkbox = if kind.has_checkbox() { "[ ] " } else { "" };
    let newline = newline_of(body);
    let line = format!("- {checkbox}{id} ({date}): {}{newline}", text.trim());
    if let Some(range) = section(body, title)? {
        let end = body[..range.end].trim_end_matches(['\r', '\n']).len();
        let separator = if body[range].trim().is_empty() {
            format!("{newline}{newline}")
        } else {
            newline.to_owned()
        };
        body.insert_str(end, &format!("{separator}{line}"));
    } else {
        write!(body, "{newline}## {title}{newline}{newline}{line}")?;
    }
    ensure!(
        located_items(body, kind)?
            .iter()
            .any(|entry| entry.item.id.as_deref() == Some(&id)),
        "Could not insert the item; check for unclosed code or HTML blocks"
    );
    Ok(id)
}

/// Checks a question (with its answer) or a debt (without one).
pub fn complete_item(
    body: &mut String,
    kind: ItemKind,
    id: &str,
    answer: Option<&str>,
    date: NaiveDate,
) -> Result<()> {
    ensure!(kind.has_checkbox(), "Decisions cannot be completed");
    ensure!(
        kind != ItemKind::Question || answer.is_some(),
        "A question requires an answer"
    );
    ensure!(kind.owns(id), "Use an ID in the form {}<n>", kind.prefix());
    if let Some(answer) = answer {
        single_line(answer)?;
    }
    let title = kind.section();
    let matches: Vec<_> = located_items(body, kind)?
        .into_iter()
        .filter(|entry| entry.item.id.as_deref() == Some(id))
        .collect();
    ensure!(
        matches.len() == 1,
        "Item '{id}' is missing or duplicated in '{title}'"
    );
    let entry = &matches[0];
    ensure!(!entry.item.done, "Item '{id}' is already completed");
    let source = &body[entry.range.clone()];
    let first_end = source.find('\n').unwrap_or(source.len());
    let first = &source[..first_end];
    let marker_end = first
        .find(char::is_whitespace)
        .context("Invalid list marker")?;
    ensure!(
        first[marker_end..].trim_start().starts_with("[ ]"),
        "Item '{id}' requires a [ ] checkbox"
    );
    let mut replacement = first
        .replacen("[ ]", "[x]", 1)
        .trim_end_matches('\r')
        .to_owned();
    match answer {
        Some(answer) => write!(replacement, " → {} (answered {date})", answer.trim())?,
        None => write!(replacement, " (resolved {date})")?,
    }
    if first.ends_with('\r') {
        replacement.push('\r');
    }
    let start = entry.range.start;
    body.replace_range(start..start + first_end, &replacement);
    Ok(())
}

pub fn single_line(text: &str) -> Result<()> {
    ensure!(
        !text.trim().is_empty() && !text.contains(['\n', '\r']),
        "Provide non-empty text on a single line"
    );
    Ok(())
}
