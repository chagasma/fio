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
            if kind.owns(id) {
                let number = &id[kind.prefix().len()..];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::assert_err;

    type Doc = Document<serde_json::Value>;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
    }

    fn assert_all_crlf(text: &str) {
        assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
    }

    #[test]
    fn document_round_trips_and_keeps_crlf() {
        let source = "---\r\ntitle: A\r\n---\r\n\r\n## Objective\r\n";
        let document = Doc::parse(source).unwrap();
        assert_eq!(document.metadata["title"], "A");
        assert_eq!(document.render(), source);
    }

    #[test]
    fn document_rejects_missing_frontmatter() {
        assert_err(Doc::parse("# no frontmatter\n"), "Missing YAML frontmatter");
    }

    #[test]
    fn document_rejects_unclosed_frontmatter() {
        assert_err(Doc::parse("---\ntitle: A\n"), "no closing delimiter");
    }

    #[test]
    fn set_field_changes_only_that_field() {
        let mut document =
            Doc::parse("---\n# header\ntitle: A\nstatus: backlog\n---\nbody\n").unwrap();
        document.set_field("status", "done").unwrap();
        assert_eq!(document.metadata["status"], "done");
        assert_eq!(document.metadata["title"], "A");
        let rendered = document.render();
        assert!(rendered.starts_with("---\n# header\ntitle: A\nstatus:"));
        assert!(rendered.ends_with("---\nbody\n"));
    }

    #[test]
    fn set_field_keeps_the_inline_comment() {
        let mut document = Doc::parse("---\nstatus: backlog # why\n---\n").unwrap();
        document.set_field("status", "done").unwrap();
        assert!(document.render().contains("# why"));
    }

    #[test]
    fn set_field_appends_a_missing_field() {
        let mut document = Doc::parse("---\ntitle: A\n---\n").unwrap();
        document.set_field("ended", "now").unwrap();
        assert_eq!(document.metadata["ended"], "now");
        assert!(document.render().starts_with("---\ntitle: A\nended:"));
    }

    #[test]
    fn set_field_replaces_a_multiline_value_entirely() {
        let mut document = Doc::parse("---\ntitle: {a: 1,\n  b: 2}\nstatus: x\n---\n").unwrap();
        document.set_field("title", "short").unwrap();
        assert_eq!(document.metadata["title"], "short");
        assert_eq!(document.metadata["status"], "x");
    }

    #[test]
    fn section_text_is_case_insensitive() {
        let body = "\n## Objective\n\nShip it\n\n## Debts\n";
        assert_eq!(section_text(body, "objective").unwrap(), "Ship it");
    }

    #[test]
    fn section_text_is_empty_for_empty_or_missing_sections() {
        let body = "\n## Objective\n\nShip it\n\n## Debts\n";
        assert_eq!(section_text(body, "Debts").unwrap(), "");
        assert_eq!(section_text(body, "Nope").unwrap(), "");
    }

    #[test]
    fn section_text_ignores_headings_inside_code_blocks() {
        let body = "## Objective\n\n```\n## Fake\n```\n";
        assert_eq!(section_text(body, "Fake").unwrap(), "");
        assert!(section_text(body, "Objective").unwrap().contains("## Fake"));
    }

    #[test]
    fn a_level_one_heading_ends_a_section_but_level_three_stays_inside() {
        let body = "## A\n\ntext\n\n### Sub\n\nmore\n\n# Top\n\nafter\n";
        let text = section_text(body, "A").unwrap();
        assert!(text.contains("### Sub") && text.contains("more"));
        assert!(!text.contains("after"));
    }

    #[test]
    fn duplicate_sections_are_an_error() {
        assert_err(section_text("## A\n\n## A\n", "A"), "Duplicate 'A' section");
    }

    #[test]
    fn set_section_replaces_only_that_section() {
        let mut body = "\n## Objective\n\nOld\n\n## Debts\n\nkeep\n".to_owned();
        set_section(&mut body, "Objective", "New").unwrap();
        assert_eq!(section_text(&body, "Objective").unwrap(), "New");
        assert_eq!(section_text(&body, "Debts").unwrap(), "keep");
    }

    #[test]
    fn set_section_appends_a_missing_section() {
        let mut body = "\n## Objective\n\nText\n".to_owned();
        set_section(&mut body, "Current state", "Working").unwrap();
        assert_eq!(section_text(&body, "Current state").unwrap(), "Working");
        assert_eq!(section_text(&body, "Objective").unwrap(), "Text");
    }

    #[test]
    fn set_section_rejects_empty_text() {
        let mut body = "## Objective\n\nOld\n".to_owned();
        assert_err(
            set_section(&mut body, "Objective", "  \n "),
            "cannot be empty",
        );
        assert_eq!(body, "## Objective\n\nOld\n");
    }

    #[test]
    fn set_section_rejects_heading_injection() {
        let mut body = "## Objective\n\nOld\n".to_owned();
        assert_err(
            set_section(&mut body, "Objective", "text\n## Other"),
            "another level 1 or 2 heading",
        );
        assert_eq!(body, "## Objective\n\nOld\n");
    }

    #[test]
    fn set_section_converts_text_to_the_body_newline_style() {
        let mut body = "\r\n## Objective\r\n\r\nOld\r\n".to_owned();
        set_section(&mut body, "Objective", "one\ntwo").unwrap();
        assert_all_crlf(&body);
        assert_eq!(section_text(&body, "Objective").unwrap(), "one\r\ntwo");
    }

    #[test]
    fn add_item_writes_the_documented_line_format() {
        let mut body = String::new();
        add_item(&mut body, ItemKind::Question, "why?", date(1)).unwrap();
        assert_eq!(body, "\n## Questions\n\n- [ ] Q1 (2026-01-01): why?\n");
    }

    #[test]
    fn add_item_numbers_sequentially_per_kind() {
        let mut body = String::new();
        let mut add = |kind, text| add_item(&mut body, kind, text, date(1)).unwrap();
        assert_eq!(add(ItemKind::Question, "a"), "Q1");
        assert_eq!(add(ItemKind::Question, "b"), "Q2");
        assert_eq!(add(ItemKind::Decision, "c"), "DEC1");
        assert_eq!(add(ItemKind::Debt, "d"), "D1");
    }

    #[test]
    fn decisions_have_no_checkbox() {
        let mut body = String::new();
        add_item(&mut body, ItemKind::Decision, "use Rust", date(1)).unwrap();
        assert!(body.contains("\n- DEC1 (2026-01-01): use Rust"));
        assert!(!body.contains("[ ]"));
    }

    #[test]
    fn add_item_appends_inside_an_existing_empty_section() {
        let mut body = "\n## Questions\n\n## Debts\n".to_owned();
        add_item(&mut body, ItemKind::Question, "one", date(1)).unwrap();
        add_item(&mut body, ItemKind::Question, "two", date(1)).unwrap();
        assert_eq!(items(&body, ItemKind::Question).unwrap().len(), 2);
        assert!(items(&body, ItemKind::Debt).unwrap().is_empty());
    }

    #[test]
    fn add_item_rejects_multiline_and_blank_text() {
        let mut body = String::new();
        assert_err(
            add_item(&mut body, ItemKind::Debt, "a\nb", date(1)),
            "single line",
        );
        assert_err(
            add_item(&mut body, ItemKind::Debt, "  ", date(1)),
            "single line",
        );
        assert_eq!(body, "");
    }

    #[test]
    fn add_item_rejects_duplicate_ids() {
        let mut body = "## Debts\n\n- [ ] D1: a\n- [ ] D1: b\n".to_owned();
        assert_err(
            add_item(&mut body, ItemKind::Debt, "c", date(1)),
            "Duplicate ID in 'Debts': D1",
        );
    }

    #[test]
    fn add_item_ignores_ids_of_other_kinds_in_the_section() {
        let mut body = "## Debts\n\n- [ ] D1: a\n- DEC3: misplaced\n".to_owned();
        assert_eq!(
            add_item(&mut body, ItemKind::Debt, "b", date(1)).unwrap(),
            "D2"
        );
    }

    #[test]
    fn unnumbered_decisions_occupy_sequence_positions() {
        let mut body = "## Decisions\n\n- plain one\n- plain two\n".to_owned();
        assert_eq!(
            add_item(&mut body, ItemKind::Decision, "x", date(1)).unwrap(),
            "DEC3"
        );
    }

    #[test]
    fn items_ignore_nested_lists_and_quotes() {
        let body = "## Debts\n\n- [ ] D1: top\n  - [ ] D2: nested\n\n> - [ ] D3: quoted\n";
        let found = items(body, ItemKind::Debt).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.as_deref(), Some("D1"));
    }

    #[test]
    fn items_accept_any_list_marker_and_checkbox_case() {
        let body = "## Debts\n\n* [ ] D1: a\n+ [X] D2: b\n\n1. [x] D3: c\n";
        let found = items(body, ItemKind::Debt).unwrap();
        let summary: Vec<_> = found
            .iter()
            .map(|i| (i.id.as_deref().unwrap(), i.done))
            .collect();
        assert_eq!(summary, [("D1", false), ("D2", true), ("D3", true)]);
    }

    #[test]
    fn items_read_ids_with_and_without_a_trailing_colon() {
        let body = "## Debts\n\n- [ ] D1: legacy\n- [ ] D2 (2026-01-01): current\n";
        let ids: Vec<_> = items(body, ItemKind::Debt)
            .unwrap()
            .into_iter()
            .map(|item| item.id.unwrap())
            .collect();
        assert_eq!(ids, ["D1", "D2"]);
    }

    #[test]
    fn question_completion_requires_an_answer() {
        let mut body = String::new();
        add_item(&mut body, ItemKind::Question, "why?", date(1)).unwrap();
        assert_err(
            complete_item(&mut body, ItemKind::Question, "Q1", None, date(2)),
            "requires an answer",
        );
    }

    #[test]
    fn completing_a_question_checks_the_box_and_records_the_answer() {
        let mut body = String::new();
        add_item(&mut body, ItemKind::Question, "why?", date(1)).unwrap();
        complete_item(
            &mut body,
            ItemKind::Question,
            "Q1",
            Some("because"),
            date(2),
        )
        .unwrap();
        assert!(body.contains("- [x] Q1 (2026-01-01): why? → because (answered 2026-01-02)"));
        assert!(open_items(&body, ItemKind::Question).unwrap().is_empty());
    }

    #[test]
    fn completing_a_debt_records_the_resolution_date() {
        let mut body = String::new();
        add_item(&mut body, ItemKind::Debt, "cleanup", date(1)).unwrap();
        complete_item(&mut body, ItemKind::Debt, "D1", None, date(2)).unwrap();
        assert!(body.contains("- [x] D1 (2026-01-01): cleanup (resolved 2026-01-02)"));
    }

    #[test]
    fn complete_item_rejects_completed_unknown_wrong_kind_and_decisions() {
        let mut body = String::new();
        add_item(&mut body, ItemKind::Debt, "cleanup", date(1)).unwrap();
        add_item(&mut body, ItemKind::Decision, "choice", date(1)).unwrap();
        let mut complete = |kind, id: &str| complete_item(&mut body, kind, id, None, date(2));
        assert_err(complete(ItemKind::Debt, "D9"), "missing or duplicated");
        assert_err(complete(ItemKind::Debt, "Q1"), "Use an ID in the form D<n>");
        assert_err(
            complete(ItemKind::Decision, "DEC1"),
            "Decisions cannot be completed",
        );
        complete(ItemKind::Debt, "D1").unwrap();
        assert_err(complete(ItemKind::Debt, "D1"), "already completed");
    }

    #[test]
    fn complete_item_requires_a_checkbox() {
        let mut body = "## Debts\n\n- D1: no box\n".to_owned();
        assert_err(
            complete_item(&mut body, ItemKind::Debt, "D1", None, date(2)),
            "requires a [ ] checkbox",
        );
    }

    #[test]
    fn complete_item_preserves_crlf() {
        let mut body =
            "\r\n## Debts\r\n\r\n- [ ] D1 (2026-01-01): a\r\n- [ ] D2 (2026-01-01): b\r\n"
                .to_owned();
        complete_item(&mut body, ItemKind::Debt, "D1", None, date(2)).unwrap();
        assert_all_crlf(&body);
        assert_eq!(open_items(&body, ItemKind::Debt).unwrap().len(), 1);
    }
}
