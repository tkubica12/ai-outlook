use crate::calendar::Event;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const WORD_LIMIT: usize = 64;

#[derive(Default, Clone)]
pub(crate) struct Index {
    words: Vec<Word>,
    word_ids: BTreeMap<String, usize>,
    documents: Vec<Vec<Field>>,
}

#[derive(Clone)]
struct Word {
    key: String,
    chars: Vec<char>,
}

#[derive(Clone)]
struct Field {
    name: &'static str,
    key: String,
    words: Vec<usize>,
}

pub(crate) struct Query {
    terms: Vec<Term>,
    invalid: bool,
}

struct Term {
    query: String,
    key: String,
    candidates: BTreeMap<usize, u8>,
}

#[derive(Serialize)]
pub(crate) struct Match {
    pub approximate: bool,
    pub edit_distance: u16,
    pub terms: Vec<MatchedTerm>,
    #[serde(skip)]
    rank: (usize, u16, u16),
}

impl Match {
    pub fn rank(&self) -> (usize, u16, u16) {
        self.rank
    }
}

#[derive(Serialize)]
pub(crate) struct MatchedTerm {
    query: String,
    matched: String,
    field: &'static str,
    kind: &'static str,
    edits: u8,
}

fn folded(character: char) -> char {
    match character {
        '\u{e0}'..='\u{e5}' | '\u{101}' | '\u{103}' | '\u{105}' => 'a',
        '\u{e7}' | '\u{107}' | '\u{10d}' => 'c',
        '\u{10f}' | '\u{111}' => 'd',
        '\u{e8}'..='\u{eb}' | '\u{113}' | '\u{117}' | '\u{119}' | '\u{11b}' => 'e',
        '\u{ec}'..='\u{ef}' | '\u{12b}' => 'i',
        '\u{13a}' | '\u{13e}' | '\u{142}' => 'l',
        '\u{f1}' | '\u{144}' | '\u{148}' => 'n',
        '\u{f2}'..='\u{f6}' | '\u{f8}' | '\u{14d}' | '\u{151}' => 'o',
        '\u{155}' | '\u{159}' => 'r',
        '\u{15b}' | '\u{161}' => 's',
        '\u{165}' => 't',
        '\u{f9}'..='\u{fc}' | '\u{16b}' | '\u{16f}' | '\u{171}' => 'u',
        '\u{fd}' | '\u{ff}' => 'y',
        '\u{17a}' | '\u{17c}' | '\u{17e}' => 'z',
        other => other,
    }
}

fn combining(character: char) -> bool {
    matches!(character, '\u{300}'..='\u{36f}')
}

pub(crate) fn key(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(folded)
        .filter(|character| character.is_alphanumeric() && !combining(*character))
        .collect()
}

fn words(text: &str) -> Vec<String> {
    let text = text
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| !combining(*character))
        .map(folded)
        .collect::<String>();
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn tolerance(chars: &[char]) -> u8 {
    if chars.iter().any(|character| !character.is_alphabetic()) {
        return 0;
    }
    match chars.len() {
        4..=9 => 1,
        10..=WORD_LIMIT => 2,
        _ => 0,
    }
}

// Banded optimal-string-alignment distance counts an adjacent swap as one typo.
fn distance(left: &[char], right: &[char], limit: u8) -> Option<u8> {
    if left.len() > WORD_LIMIT
        || right.len() > WORD_LIMIT
        || left.len().abs_diff(right.len()) > usize::from(limit)
    {
        return None;
    }
    let beyond = limit + 1;
    let mut previous_previous = [beyond; WORD_LIMIT + 1];
    let mut previous = [beyond; WORD_LIMIT + 1];
    for (index, value) in previous.iter_mut().enumerate().take(right.len() + 1) {
        *value = (index as u8).min(beyond);
    }
    for row in 1..=left.len() {
        let mut current = [beyond; WORD_LIMIT + 1];
        current[0] = (row as u8).min(beyond);
        let first = row.saturating_sub(usize::from(limit)).max(1);
        let last = (row + usize::from(limit)).min(right.len());
        for column in first..=last {
            current[column] = (previous[column] + 1)
                .min(current[column - 1] + 1)
                .min(previous[column - 1] + u8::from(left[row - 1] != right[column - 1]));
            if row > 1
                && column > 1
                && left[row - 1] == right[column - 2]
                && left[row - 2] == right[column - 1]
            {
                current[column] = current[column].min(previous_previous[column - 2] + 1);
            }
        }
        if current[..=right.len()].iter().all(|value| *value > limit) {
            return None;
        }
        previous_previous = previous;
        previous = current;
    }
    (previous[right.len()] <= limit).then_some(previous[right.len()])
}

impl Index {
    pub fn build(events: &[Event]) -> Self {
        let mut index = Self::default();
        for event in events {
            let mut fields = Vec::new();
            for (name, text) in [
                ("title", event.title.as_str()),
                ("organizer", event.organizer.as_str()),
                ("location", event.location.as_str()),
                ("category", event.category.as_str()),
            ]
            .into_iter()
            .chain(
                event
                    .attendees
                    .iter()
                    .map(|text| ("attendee", text.as_str())),
            ) {
                let parts = words(text);
                let mut ids = BTreeSet::new();
                for start in 0..parts.len() {
                    let mut compound = String::new();
                    for part in parts[start..].iter().take(3) {
                        compound.push_str(part);
                        if compound.chars().count() > WORD_LIMIT {
                            break;
                        }
                        let next = index.words.len();
                        let id = *index.word_ids.entry(compound.clone()).or_insert(next);
                        if id == next {
                            index.words.push(Word {
                                key: compound.clone(),
                                chars: compound.chars().collect(),
                            });
                        }
                        ids.insert(id);
                    }
                }
                fields.push(Field {
                    name,
                    key: key(text),
                    words: ids.into_iter().collect(),
                });
            }
            index.documents.push(fields);
        }
        index
    }

    pub fn query(&self, text: &str) -> Query {
        let mut seen = BTreeSet::new();
        let mut query = Query {
            terms: Vec::new(),
            invalid: false,
        };
        for term in text.split_whitespace() {
            let key = key(term);
            if key.is_empty() {
                continue;
            }
            if !seen.insert(key.clone()) {
                continue;
            }
            let chars = key.chars().collect::<Vec<_>>();
            let limit = tolerance(&chars);
            let mut candidates = BTreeMap::new();
            if let Some(id) = self.word_ids.get(&key) {
                candidates.insert(*id, 0);
            }
            if limit > 0 {
                for (id, word) in self.words.iter().enumerate() {
                    if tolerance(&word.chars) > 0
                        && let Some(edits) = distance(&chars, &word.chars, limit)
                    {
                        candidates.insert(id, edits);
                    }
                }
            }
            query.terms.push(Term {
                query: term.into(),
                key,
                candidates,
            });
        }
        query.invalid = !text.trim().is_empty() && query.terms.is_empty();
        query
    }

    pub fn matches(&self, index: usize, query: &Query) -> Option<Match> {
        if query.invalid {
            return None;
        }
        let mut result = Match {
            approximate: false,
            edit_distance: 0,
            terms: Vec::new(),
            rank: (0, 0, 0),
        };
        for term in &query.terms {
            let mut best = None;
            for field in &self.documents[index] {
                let field_cost = match field.name {
                    "title" => 0,
                    "organizer" | "attendee" => 1,
                    _ => 2,
                };
                if field.key.contains(&term.key) {
                    let cost = (0, 0, 4 + field_cost);
                    if best.as_ref().is_none_or(|(prior, _)| cost < *prior) {
                        best = Some((
                            cost,
                            MatchedTerm {
                                query: term.query.clone(),
                                matched: term.key.clone(),
                                field: field.name,
                                kind: "substring",
                                edits: 0,
                            },
                        ));
                    }
                }
                for id in &field.words {
                    if let Some(edits) = term.candidates.get(id) {
                        let length_cost = term
                            .key
                            .chars()
                            .count()
                            .abs_diff(self.words[*id].chars.len())
                            as u16;
                        let cost = (
                            usize::from(*edits > 0),
                            u16::from(*edits),
                            field_cost + length_cost,
                        );
                        if best.as_ref().is_none_or(|(prior, _)| cost < *prior) {
                            best = Some((
                                cost,
                                MatchedTerm {
                                    query: term.query.clone(),
                                    matched: self.words[*id].key.clone(),
                                    field: field.name,
                                    kind: if *edits == 0 { "normalized" } else { "fuzzy" },
                                    edits: *edits,
                                },
                            ));
                        }
                    }
                }
            }
            let (cost, matched) = best?;
            result.rank.0 += cost.0;
            result.rank.1 += cost.1;
            result.rank.2 += cost.2;
            result.terms.push(matched);
        }
        result.approximate = result.rank.0 > 0;
        result.edit_distance = result.rank.1;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_distance_handles_edits_swaps_and_unicode() {
        for (left, right, expected) in [
            ("drmxa", "drmax", 1),
            ("workhsop", "workshop", 1),
            ("workshp", "workshop", 1),
            ("workshoap", "workshop", 1),
            ("workshap", "workshop", 1),
            ("hackaton", "hackathon", 1),
            ("hackatno", "hackathon", 2),
            ("k\u{16f}\u{148}", "k\u{16f}n", 1),
        ] {
            let left = left.chars().collect::<Vec<_>>();
            let right = right.chars().collect::<Vec<_>>();
            assert_eq!(distance(&left, &right, 2), Some(expected));
            assert_eq!(distance(&right, &left, 2), Some(expected));
            assert!(distance(&left, &right, expected - 1).is_none());
        }
        assert!(distance(&['a'; 65], &['a'; 65], 2).is_none());
        assert!(distance(&['a'; 8], &['b'; 8], 2).is_none());
    }

    #[test]
    fn normalization_handles_czech_and_decomposed_accents_without_inventing_aliases() {
        assert_eq!(key("\u{10c}ESK\u{dd} T\u{dd}M"), "ceskytym");
        assert_eq!(key("c\u{30c}esky\u{301} ty\u{301}m"), "ceskytym");
        assert_eq!(key("North. Star"), "northstar");
        assert_eq!(words("Dr. Example"), ["dr", "example"]);
        assert_eq!(tolerance(&['a'; 3]), 0);
        assert_eq!(tolerance(&['1'; 8]), 0);
    }

    #[test]
    fn banded_distance_agrees_with_a_full_matrix_on_small_edit_combinations() {
        fn reference(left: &[char], right: &[char]) -> u8 {
            let mut rows = vec![vec![0u8; right.len() + 1]; left.len() + 1];
            for (index, row) in rows.iter_mut().enumerate() {
                row[0] = index as u8;
            }
            for (index, value) in rows[0].iter_mut().enumerate() {
                *value = index as u8;
            }
            for row in 1..=left.len() {
                for column in 1..=right.len() {
                    rows[row][column] = (rows[row - 1][column] + 1)
                        .min(rows[row][column - 1] + 1)
                        .min(
                            rows[row - 1][column - 1]
                                + u8::from(left[row - 1] != right[column - 1]),
                        );
                    if row > 1
                        && column > 1
                        && left[row - 1] == right[column - 2]
                        && left[row - 2] == right[column - 1]
                    {
                        rows[row][column] = rows[row][column].min(rows[row - 2][column - 2] + 1);
                    }
                }
            }
            rows[left.len()][right.len()]
        }
        let mut samples = vec![Vec::new()];
        for length in 1..=4 {
            for encoded in 0..3usize.pow(length) {
                let mut encoded = encoded;
                samples.push(
                    (0..length)
                        .map(|_| {
                            let character = ['a', 'b', 'c'][encoded % 3];
                            encoded /= 3;
                            character
                        })
                        .collect(),
                );
            }
        }
        for left in &samples {
            for right in &samples {
                let expected = reference(left, right);
                for limit in 0..=2 {
                    assert_eq!(
                        distance(left, right, limit),
                        (expected <= limit).then_some(expected),
                        "{left:?}, {right:?}, {limit}"
                    );
                }
            }
        }
    }
}
