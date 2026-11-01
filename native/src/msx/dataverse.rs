//! Dataverse (MSX) query building, escaping, parsing and customer matching.
use super::{Client, DATAVERSE_ORIGIN, Resource, Source, Sourced, mcp};
use chrono::{NaiveDate, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Tables the model may query through `tomlook_msx_query`.
pub const AI_TABLES: [&str; 6] = [
    "account",
    "contact",
    "msp_engagementmilestone",
    "opportunity",
    "systemuser",
    "task",
];
pub const AI_ROW_LIMIT: usize = 50;
pub const INVALID_DOMAIN: &str = "Not a valid customer e-mail domain";
pub const INVALID_NAME: &str = "Not a valid company name";
/// Consumer mail providers; never a customer domain.
pub const PUBLIC_MAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "gmx.com",
    "gmx.net",
    "gmx.de",
    "web.de",
    "seznam.cz",
    "email.cz",
    "centrum.cz",
    "atlas.cz",
    "volny.cz",
    "post.cz",
];
const LEGAL_FORMS: &[&str] = &[
    "inc",
    "incorporated",
    "ltd",
    "limited",
    "llc",
    "gmbh",
    "ag",
    "corp",
    "corporation",
    "co",
    "company",
    "plc",
    "sa",
    "se",
    "bv",
    "nv",
    "kg",
    "spol",
    "sro",
    "as",
    "oy",
    "ab",
    "srl",
    "spa",
];
const CLOSED_MILESTONE: &[&str] = &[
    "complete",
    "cancel",
    "closed",
    "duplicate",
    "hygiene",
    "not pursued",
];

/// A single-quoted SQL literal; quotes are doubled.
pub fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A `LIKE '%value%'` literal with SQL wildcards escaped.
pub fn like_contains(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('%');
    for character in value.chars() {
        match character {
            '[' => escaped.push_str("[[]"),
            '%' => escaped.push_str("[%]"),
            '_' => escaped.push_str("[_]"),
            character => escaped.push(character),
        }
    }
    escaped.push('%');
    sql_string(&escaped)
}

/// A lowercase ASCII (IDNA) host name, or an error for anything that is not a plain domain.
pub fn domain(input: &str) -> Result<String, String> {
    let trimmed = input.trim().trim_start_matches('@').trim_end_matches('.');
    if trimmed.is_empty()
        || trimmed.len() > 253
        || trimmed.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || "'\"`;%_[]\\/:@?#*()=<>,&|+!$^{}~".contains(character)
        })
    {
        return Err(INVALID_DOMAIN.into());
    }
    let ascii = match url::Host::parse(trimmed) {
        Ok(url::Host::Domain(domain)) => domain.to_ascii_lowercase(),
        _ => return Err(INVALID_DOMAIN.into()),
    };
    let ascii = ascii.strip_prefix("www.").unwrap_or(&ascii).to_owned();
    let labels = ascii.split('.').collect::<Vec<_>>();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        || !labels
            .last()
            .is_some_and(|tld| tld.bytes().any(|byte| byte.is_ascii_alphabetic()))
    {
        return Err(INVALID_DOMAIN.into());
    }
    Ok(ascii)
}

pub fn company_name(input: &str) -> Result<String, String> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = name.chars().count();
    if !(2..=100).contains(&count)
        || !name
            .chars()
            .all(|character| character.is_alphanumeric() || " .,&'-+()/".contains(character))
        || !name.chars().any(char::is_alphanumeric)
    {
        return Err(INVALID_NAME.into());
    }
    Ok(name)
}

fn fold(character: char) -> char {
    match character {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ą' => 'a',
        'č' | 'ç' | 'ć' => 'c',
        'ď' => 'd',
        'é' | 'è' | 'ê' | 'ë' | 'ě' | 'ę' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ľ' | 'ĺ' | 'ł' => 'l',
        'ň' | 'ñ' | 'ń' => 'n',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
        'ř' => 'r',
        'š' | 'ś' => 's',
        'ť' => 't',
        'ú' | 'ù' | 'û' | 'ü' | 'ů' => 'u',
        'ý' | 'ÿ' => 'y',
        'ž' | 'ź' | 'ż' => 'z',
        character => character,
    }
}

/// Distinctive lowercase name tokens without diacritics, punctuation or legal forms.
pub fn name_tokens(name: &str) -> Vec<String> {
    let folded = name
        .to_lowercase()
        .chars()
        .map(fold)
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    folded
        .split_whitespace()
        .filter(|token| token.chars().count() > 1 && !LEGAL_FORMS.contains(token))
        .map(str::to_owned)
        .collect()
}

/// Token similarity in 0..=1; containment of one full name in the other scores at least 0.75.
pub fn similarity(left: &str, right: &str) -> f32 {
    let left = name_tokens(left).into_iter().collect::<BTreeSet<_>>();
    let right = name_tokens(right).into_iter().collect::<BTreeSet<_>>();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let shared = left.intersection(&right).count() as f32;
    let jaccard = shared / left.union(&right).count() as f32;
    if shared > 0.0 && (left.is_subset(&right) || right.is_subset(&left)) {
        jaccard.max(0.75)
    } else {
        jaccard
    }
}

fn website_host(website: &str) -> Option<String> {
    let website = website.trim();
    let url = if website.contains("://") {
        website.to_owned()
    } else {
        format!("https://{website}")
    };
    let host = url::Url::parse(&url).ok()?.host_str()?.to_ascii_lowercase();
    Some(host.strip_prefix("www.").unwrap_or(&host).to_owned())
}

fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}")) || domain.ends_with(&format!(".{host}"))
}

fn domain_stem(domain: &str) -> Option<&str> {
    let labels = domain.split('.').collect::<Vec<_>>();
    let index = if labels.len() >= 3 && labels[labels.len() - 2].len() <= 3 {
        labels.len() - 3
    } else {
        labels.len().checked_sub(2)?
    };
    labels.get(index).copied().filter(|stem| stem.len() >= 3)
}

/// The MSX web URL of one Dataverse record.
pub fn record_url(table: &str, id: &str) -> String {
    format!("{DATAVERSE_ORIGIN}/main.aspx?etn={table}&id={id}&pagetype=entityrecord")
}

// ---------- row access ----------

fn field<'a>(row: &'a Value, key: &str) -> Option<&'a Value> {
    let object = row.as_object()?;
    object
        .get(key)
        .or_else(|| {
            object
                .iter()
                .find(|(name, _)| {
                    name.eq_ignore_ascii_case(key)
                        || name
                            .rsplit_once('.')
                            .is_some_and(|(_, column)| column.eq_ignore_ascii_case(key))
                })
                .map(|(_, value)| value)
        })
        .filter(|value| !value.is_null())
}

pub(super) fn text(row: &Value, key: &str) -> Option<String> {
    let value = match field(row, key)? {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    let value = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(400)
        .collect::<String>();
    (!value.is_empty()).then_some(value)
}

fn formatted(row: &Value, key: &str) -> Option<String> {
    text(
        row,
        &format!("{key}@OData.Community.Display.V1.FormattedValue"),
    )
    .or_else(|| text(row, &format!("{key}name")))
}

pub(super) fn guid(row: &Value, key: &str) -> Option<String> {
    let value = text(row, key)?;
    let value = value.trim_matches(|character| character == '{' || character == '}');
    super::auth::uuid(value).then(|| value.to_ascii_lowercase())
}

pub(super) fn date(row: &Value, key: &str) -> Option<NaiveDate> {
    let value = text(row, key)?;
    NaiveDate::parse_from_str(value.get(..10)?, "%Y-%m-%d")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(value.split_whitespace().next()?, "%m/%d/%Y").ok())
}

fn number(row: &Value, key: &str) -> Option<f64> {
    match field(row, key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

// ---------- schema discovery ----------

/// Logical column names found in a `describe` result (Markdown or JSON).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableSchema {
    pub columns: BTreeSet<String>,
}

fn identifier(text: &str) -> Option<String> {
    let text = text
        .trim()
        .trim_matches(|character| matches!(character, '`' | '*' | '"' | '\''));
    (text.len() <= 100
        && text.starts_with(|character: char| character.is_ascii_lowercase())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    .then(|| text.to_owned())
}

impl TableSchema {
    pub fn parse(value: &Value) -> Self {
        let mut columns = BTreeSet::new();
        Self::walk(value, &mut columns, 0);
        Self { columns }
    }

    fn walk(value: &Value, columns: &mut BTreeSet<String>, depth: usize) {
        if depth > 12 || columns.len() > 4000 {
            return;
        }
        match value {
            Value::String(text) => Self::text(text, columns),
            Value::Array(items) => {
                for item in items {
                    Self::walk(item, columns, depth + 1);
                }
            }
            Value::Object(object) => {
                for (key, item) in object {
                    let key = key.to_ascii_lowercase();
                    if let (true, Some(name)) = (
                        matches!(
                            key.as_str(),
                            "logicalname"
                                | "logical_name"
                                | "columnname"
                                | "column"
                                | "field"
                                | "name"
                                | "attribute"
                        ),
                        item.as_str().and_then(identifier),
                    ) {
                        columns.insert(name);
                    } else {
                        Self::walk(item, columns, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    fn text(text: &str, columns: &mut BTreeSet<String>) {
        for line in text.lines().take(20_000) {
            let line = line.trim();
            if let Some(cell) = line
                .strip_prefix('|')
                .and_then(|row| row.split('|').next())
                .and_then(identifier)
            {
                columns.insert(cell);
            }
            if let Some(item) = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .and_then(|item| item.split([' ', ':', '(']).next())
                .and_then(identifier)
            {
                columns.insert(item);
            }
            for quoted in line.split('`').skip(1).step_by(2) {
                if let Some(name) = identifier(quoted) {
                    columns.insert(name);
                }
            }
        }
    }

    /// Enough columns were recognized to trust absence of an optional column.
    pub fn known(&self) -> bool {
        self.columns.len() >= 3
    }

    pub fn has(&self, column: &str) -> bool {
        self.columns.contains(column)
    }

    /// Required columns, standard columns the schema confirms (all when unknown), and optional
    /// MSX columns only when the schema lists them.
    pub fn select(&self, required: &[&str], standard: &[&str], optional: &[&str]) -> Vec<String> {
        required
            .iter()
            .chain(
                standard
                    .iter()
                    .filter(|column| !self.known() || self.has(column)),
            )
            .chain(optional.iter().filter(|column| self.has(column)))
            .map(|column| (*column).to_owned())
            .collect()
    }
}

// ---------- AI query validation ----------

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Number(u64),
    Text,
    Symbol(char),
}

/// Tokens with their byte ranges in the query.
fn tokens(sql: &str) -> Result<Vec<(Token, std::ops::Range<usize>)>, String> {
    let mut tokens = Vec::new();
    let mut characters = sql.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        let next = characters.peek().map(|(_, next)| *next);
        let token = match character {
            character if character.is_whitespace() => continue,
            '\'' => loop {
                match characters.next().map(|(_, character)| character) {
                    Some('\'') if characters.peek().map(|(_, next)| *next) == Some('\'') => {
                        characters.next();
                    }
                    Some('\'') => break Token::Text,
                    Some(character) if character.is_control() && character != '\t' => {
                        return Err("Query text contains control characters".into());
                    }
                    Some(_) => {}
                    None => return Err("Query has an unterminated string literal".into()),
                }
            },
            character if character.is_ascii_alphanumeric() || character == '_' => {
                let mut word = String::from(character);
                while let Some((_, next)) = characters
                    .peek()
                    .filter(|(_, next)| next.is_ascii_alphanumeric() || *next == '_')
                {
                    word.push(*next);
                    characters.next();
                }
                match word.parse() {
                    Ok(number) => Token::Number(number),
                    Err(_) => Token::Word(word.to_ascii_lowercase()),
                }
            }
            '-' if next == Some('-') => {
                return Err("Comments are not allowed in MSX queries".into());
            }
            '/' if next == Some('*') => {
                return Err("Comments are not allowed in MSX queries".into());
            }
            '(' | ')' | ',' | '.' | '=' | '<' | '>' | '!' | '*' | '+' | '-' | '/' | '%' => {
                Token::Symbol(character)
            }
            _ => {
                return Err(
                    "MSX queries allow only plain SELECT syntax (no ;, quotes other than ', brackets or variables)"
                        .into(),
                );
            }
        };
        let end = characters.peek().map_or(sql.len(), |(index, _)| *index);
        tokens.push((token, start..end));
        if tokens.len() > 2000 {
            return Err("MSX query is too long".into());
        }
    }
    Ok(tokens)
}

const FORBIDDEN: &[&str] = &[
    "insert",
    "update",
    "delete",
    "drop",
    "alter",
    "exec",
    "execute",
    "merge",
    "truncate",
    "into",
    "create",
    "grant",
    "revoke",
    "deny",
    "declare",
    "set",
    "union",
    "with",
    "waitfor",
    "shutdown",
    "dbcc",
    "backup",
    "restore",
    "bulk",
    "openrowset",
    "opendatasource",
    "openquery",
    "openxml",
    "reconfigure",
    "kill",
    "use",
    "apply",
    "except",
    "intersect",
];

const COMMA_TABLES: &str = "Use JOIN ... ON instead of comma-separated tables";
const UNBALANCED: &str = "The query has unbalanced parentheses";
/// Words that end the FROM clause.
const CLAUSES: [&str; 4] = ["where", "group", "order", "having"];
/// Keywords that are never a table alias.
const KEYWORDS: &[&str] = &[
    "as", "on", "join", "inner", "left", "right", "full", "outer", "cross", "where", "group",
    "order", "having", "from", "select", "top", "distinct", "all",
];

fn word(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index) {
        Some(Token::Word(word)) => Some(word),
        _ => None,
    }
}

/// The index after a JOIN operator starting at `index`, and whether it is a CROSS JOIN.
fn join_operator(tokens: &[Token], index: usize) -> Option<(usize, bool)> {
    match word(tokens, index)? {
        "join" => Some((index + 1, false)),
        "inner" if word(tokens, index + 1) == Some("join") => Some((index + 2, false)),
        "cross" if word(tokens, index + 1) == Some("join") => Some((index + 2, true)),
        "left" | "right" | "full" => match word(tokens, index + 1) {
            Some("join") => Some((index + 2, false)),
            Some("outer") if word(tokens, index + 2) == Some("join") => Some((index + 3, false)),
            _ => None,
        },
        _ => None,
    }
}

fn clause_end(tokens: &[Token], index: usize) -> bool {
    index >= tokens.len() || word(tokens, index).is_some_and(|word| CLAUSES.contains(&word))
}

/// Parses `table [[AS] alias] { JOIN table [[AS] alias] ON condition }` starting at `index`,
/// records every table and returns the index after the FROM clause. Each table must be an
/// unqualified allowlisted name; a top-level comma anywhere in the clause is rejected.
fn from_clause(
    tokens: &[Token],
    mut index: usize,
    tables: &mut Vec<String>,
) -> Result<usize, String> {
    let mut condition = false;
    loop {
        let table = match tokens.get(index) {
            Some(Token::Word(table)) if AI_TABLES.contains(&table.as_str()) => table,
            _ => return Err(format!("Query only these tables: {}", AI_TABLES.join(", "))),
        };
        if tokens.get(index + 1) == Some(&Token::Symbol('.')) {
            return Err("Use unqualified MSX table names without a schema or database".into());
        }
        if !tables.contains(table) {
            tables.push(table.clone());
        }
        index += 1;
        let aliased = word(tokens, index) == Some("as");
        index += usize::from(aliased);
        match word(tokens, index) {
            Some(alias) if !KEYWORDS.contains(&alias) => index += 1,
            _ if aliased => return Err("AS must be followed by a table alias".into()),
            _ => {}
        }
        if condition {
            if word(tokens, index) != Some("on") {
                return Err("Every JOIN except CROSS JOIN needs an ON condition".into());
            }
            index += 1;
            let start = index;
            let mut depth = 0_usize;
            while let Some(token) = tokens.get(index) {
                if depth == 0
                    && (clause_end(tokens, index) || join_operator(tokens, index).is_some())
                {
                    break;
                }
                match token {
                    Token::Symbol('(') => depth += 1,
                    Token::Symbol(')') => depth = depth.checked_sub(1).ok_or(UNBALANCED)?,
                    Token::Symbol(',') if depth == 0 => return Err(COMMA_TABLES.into()),
                    _ => {}
                }
                index += 1;
            }
            if index == start || depth != 0 {
                return Err("Every JOIN except CROSS JOIN needs a complete ON condition".into());
            }
        }
        if clause_end(tokens, index) {
            return Ok(index);
        }
        match join_operator(tokens, index) {
            Some((next, cross)) => {
                index = next;
                condition = !cross;
            }
            None if tokens.get(index) == Some(&Token::Symbol(',')) => {
                return Err(COMMA_TABLES.into());
            }
            None => {
                return Err(
                    "Use FROM table [alias] with JOIN ... ON; other table sources are not allowed"
                        .into(),
                );
            }
        }
    }
}

/// Validates a model-written read query: one SELECT over the MSX allowlisted tables without
/// subqueries, comments, statements or write keywords. Returns the query with a `TOP` of at most
/// 50 rows and the referenced tables.
pub fn validate_ai_query(sql: &str) -> Result<(String, Vec<String>), String> {
    let sql = sql.trim();
    if sql.is_empty() || sql.len() > 4000 {
        return Err("Write one SELECT query of at most 4,000 bytes".into());
    }
    let (tokens, spans): (Vec<_>, Vec<_>) = tokens(sql)?.into_iter().unzip();
    if tokens.first() != Some(&Token::Word("select".into())) {
        return Err("Only SELECT queries are allowed".into());
    }
    let mut depth = 0_usize;
    let mut from = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Symbol('(') => depth += 1,
            Token::Symbol(')') => depth = depth.checked_sub(1).ok_or(UNBALANCED)?,
            Token::Word(word) => {
                if FORBIDDEN.contains(&word.as_str())
                    || word.starts_with("xp_")
                    || word.starts_with("sp_")
                    || word.starts_with("fn_")
                {
                    return Err(format!("'{word}' is not allowed in a read-only MSX query"));
                }
                if word == "select" && index > 0 {
                    return Err(
                        "Subqueries and multiple statements are not allowed in MSX queries".into(),
                    );
                }
                if word == "from" {
                    if depth > 0 || from.is_some() {
                        return Err("Use exactly one top-level FROM clause".into());
                    }
                    from = Some(index);
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(UNBALANCED.into());
    }
    let from = from.ok_or("The query must select FROM an MSX table")?;
    let mut tables = Vec::new();
    let end = from_clause(&tokens, from + 1, &mut tables)?;
    if tokens.iter().enumerate().any(|(index, token)| {
        (index < from || index >= end) && matches!(token, Token::Word(word) if word == "join")
    }) {
        return Err("JOIN is allowed only in the FROM clause".into());
    }
    let modifier = word(&tokens, 1).filter(|word| matches!(*word, "distinct" | "all"));
    let at = 1 + usize::from(modifier.is_some());
    let limited = if word(&tokens, at) == Some("top") {
        let (count, span, after) =
            match (tokens.get(at + 1), tokens.get(at + 2), tokens.get(at + 3)) {
                (Some(Token::Number(count)), _, _) => (*count, spans[at + 1].clone(), at + 2),
                (
                    Some(Token::Symbol('(')),
                    Some(Token::Number(count)),
                    Some(Token::Symbol(')')),
                ) => (*count, spans[at + 2].clone(), at + 4),
                _ => return Err(format!("Use TOP followed by a number up to {AI_ROW_LIMIT}")),
            };
        if count == 0 || word(&tokens, after) == Some("percent") {
            return Err(format!(
                "TOP must be a row count between 1 and {AI_ROW_LIMIT}"
            ));
        }
        if count > AI_ROW_LIMIT as u64 {
            format!("{}{AI_ROW_LIMIT}{}", &sql[..span.start], &sql[span.end..])
        } else {
            sql.to_owned()
        }
    } else {
        let head = match modifier {
            Some(modifier) => format!("SELECT {}", modifier.to_ascii_uppercase()),
            None => "SELECT".to_owned(),
        };
        format!(
            "{head} TOP {AI_ROW_LIMIT} {}",
            sql[spans[at - 1].end..].trim_start()
        )
    };
    Ok((limited, tables))
}

// ---------- typed records ----------

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SystemUser {
    pub id: String,
    pub name: Option<String>,
    pub email: Option<String>,
    /// How the signed-in identity was matched: "Entra object ID" or "sign-in name".
    pub matched_by: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Owner {
    pub id: Option<String>,
    pub name: Option<String>,
}

fn owner(row: &Value) -> Option<Owner> {
    let owner = Owner {
        id: guid(row, "ownerid"),
        name: text(row, "owneridname").or_else(|| formatted(row, "ownerid")),
    };
    (owner.id.is_some() || owner.name.is_some()).then_some(owner)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    WebsiteDomain { domain: String, website: String },
    ContactDomain { domain: String, contacts: usize },
    NameSimilarity { name: String, similarity: f32 },
    DomainMatchesName { domain: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AccountCandidate {
    pub id: String,
    pub name: String,
    /// MSX top parent ID (TPID) when the account schema exposes it.
    pub tpid: Option<String>,
    pub account_number: Option<String>,
    pub website: Option<String>,
    pub owner: Option<Owner>,
    pub url: String,
    pub confidence: f32,
    pub evidence: Vec<Evidence>,
}

const TPID_COLUMNS: [&str; 3] = ["msp_mstopparentid", "msp_tpid", "msp_accounttpid"];

pub(super) fn account(row: &Value) -> Option<AccountCandidate> {
    let id = guid(row, "accountid")?;
    Some(AccountCandidate {
        url: record_url("account", &id),
        id,
        name: text(row, "name")?,
        tpid: TPID_COLUMNS
            .iter()
            .find_map(|column| text(row, column))
            .filter(|tpid| tpid.len() <= 20 && tpid.bytes().all(|byte| byte.is_ascii_digit())),
        account_number: text(row, "accountnumber"),
        website: text(row, "websiteurl"),
        owner: owner(row),
        confidence: 0.0,
        evidence: Vec::new(),
    })
}

/// Scores one candidate from its matching evidence. Inferred matches never reach certainty.
pub fn score(
    candidate: &mut AccountCandidate,
    domains: &[String],
    names: &[String],
    contacts: &BTreeMap<String, usize>,
) {
    let mut confidence = 0.0_f32;
    let mut evidence = Vec::new();
    if let Some((website, host)) = candidate
        .website
        .as_ref()
        .and_then(|website| Some((website, website_host(website)?)))
        && let Some(domain) = domains.iter().find(|domain| domain_matches(&host, domain))
    {
        confidence += 0.55;
        evidence.push(Evidence::WebsiteDomain {
            domain: domain.clone(),
            website: website.clone(),
        });
    }
    if let Some((domain, count)) = contacts.iter().max_by_key(|(_, count)| **count) {
        confidence += 0.35 + 0.05 * ((*count).clamp(1, 5) - 1) as f32;
        evidence.push(Evidence::ContactDomain {
            domain: domain.clone(),
            contacts: *count,
        });
    }
    if let Some((name, similarity)) = names
        .iter()
        .map(|name| (name, similarity(name, &candidate.name)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, similarity)| *similarity >= 0.5)
    {
        confidence += 0.4 * similarity;
        evidence.push(Evidence::NameSimilarity {
            name: name.clone(),
            similarity,
        });
    }
    let tokens = name_tokens(&candidate.name);
    if let Some(domain) = domains.iter().find(|domain| {
        domain_stem(domain).is_some_and(|stem| tokens.iter().any(|token| token == stem))
    }) {
        confidence += 0.15;
        evidence.push(Evidence::DomainMatchesName {
            domain: domain.clone(),
        });
    }
    candidate.confidence = (confidence.min(0.95) * 100.0).round() / 100.0;
    candidate.evidence = evidence;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selection {
    /// Clearly ahead of every alternative; still an inference that the user must confirm.
    Likely {
        account_id: String,
    },
    Ambiguous,
    NoMatch,
}

pub fn select(candidates: &[AccountCandidate]) -> Selection {
    match candidates {
        [] => Selection::NoMatch,
        [first, rest @ ..]
            if first.confidence >= 0.6
                && rest
                    .first()
                    .is_none_or(|second| first.confidence - second.confidence >= 0.2) =>
        {
            Selection::Likely {
                account_id: first.id.clone(),
            }
        }
        _ => Selection::Ambiguous,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountQuery {
    /// Customer e-mail domains of meeting participants (not the user's own organization).
    pub domains: Vec<String>,
    pub names: Vec<String>,
}

impl AccountQuery {
    /// Valid, distinct inputs (at most five domains and three names) and the rejected ones.
    pub fn prepare(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut rejected = Vec::new();
        let mut domains = Vec::new();
        for input in self.domains.iter().take(20) {
            match domain(input) {
                Ok(domain) if PUBLIC_MAIL.contains(&domain.as_str()) => {
                    rejected.push(format!("{domain}: public mail domain"))
                }
                Ok(domain) if !domains.contains(&domain) && domains.len() < 5 => {
                    domains.push(domain)
                }
                Ok(_) => {}
                Err(error) => rejected.push(format!(
                    "{}: {error}",
                    input.chars().take(60).collect::<String>()
                )),
            }
        }
        let mut names = Vec::new();
        for input in self.names.iter().take(10) {
            match company_name(input) {
                Ok(name) if !names.contains(&name) && names.len() < 3 => names.push(name),
                Ok(_) => {}
                Err(error) => rejected.push(format!(
                    "{}: {error}",
                    input.chars().take(60).collect::<String>()
                )),
            }
        }
        (domains, names, rejected)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AccountLookup {
    pub candidates: Vec<AccountCandidate>,
    pub selection: Selection,
    /// Customer associations are inferred; the user confirms before they are treated as fact.
    pub requires_confirmation: bool,
    pub rejected_inputs: Vec<String>,
    pub sources: Vec<Source>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Opportunity {
    pub id: String,
    pub name: String,
    pub stage: Option<String>,
    pub estimated_close: Option<NaiveDate>,
    pub estimated_value: Option<f64>,
    pub owner: Option<Owner>,
    pub url: String,
}

pub(super) fn opportunity(row: &Value) -> Option<Opportunity> {
    let id = guid(row, "opportunityid")?;
    Some(Opportunity {
        url: record_url("opportunity", &id),
        id,
        name: text(row, "name")?,
        stage: text(row, "stepname")
            .or_else(|| formatted(row, "msp_salesstagecode"))
            .or_else(|| formatted(row, "salesstage")),
        estimated_close: date(row, "estimatedclosedate"),
        estimated_value: number(row, "estimatedvalue"),
        owner: owner(row),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Milestone {
    pub id: String,
    pub name: String,
    pub due: Option<NaiveDate>,
    pub status: Option<String>,
    pub status_code: Option<i64>,
    pub workload: Option<String>,
    pub opportunity_id: Option<String>,
    pub owner: Option<Owner>,
    pub url: String,
}

pub(super) fn milestone(row: &Value) -> Option<Milestone> {
    let id = guid(row, "msp_engagementmilestoneid")?;
    Some(Milestone {
        url: record_url("msp_engagementmilestone", &id),
        id,
        name: text(row, "msp_name")?,
        due: date(row, "msp_milestonedate"),
        status: formatted(row, "msp_milestonestatus"),
        status_code: number(row, "msp_milestonestatus")
            .filter(|value| value.fract() == 0.0)
            .map(|value| value as i64),
        workload: formatted(row, "msp_workload").or_else(|| text(row, "msp_workload")),
        opportunity_id: guid(row, "msp_opportunityid"),
        owner: owner(row),
    })
}

/// Completed, cancelled and hygiene milestones are excluded when their status label is known.
pub fn open_milestone(milestone: &Milestone) -> bool {
    milestone.status.as_ref().is_none_or(|status| {
        let status = status.to_ascii_lowercase();
        !CLOSED_MILESTONE
            .iter()
            .any(|closed| status.contains(closed))
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnedTask {
    pub id: String,
    pub subject: String,
    pub due: Option<NaiveDate>,
    pub state: Option<String>,
    pub milestone_id: String,
    /// The returned regarding type was checked to be an engagement milestone.
    pub regarding_type_verified: bool,
    pub url: String,
}

pub(super) fn owned_task(row: &Value, milestones: &BTreeSet<String>) -> Option<OwnedTask> {
    let milestone_id = guid(row, "regardingobjectid").filter(|id| milestones.contains(id))?;
    let regarding_type = text(row, "regardingobjecttypecode").or_else(|| {
        text(
            row,
            "regardingobjectid@Microsoft.Dynamics.CRM.lookuplogicalname",
        )
    });
    if regarding_type
        .as_ref()
        .is_some_and(|kind| kind != "msp_engagementmilestone")
    {
        return None;
    }
    let id = guid(row, "activityid")?;
    Some(OwnedTask {
        url: record_url("task", &id),
        id,
        subject: text(row, "subject").unwrap_or_default(),
        due: date(row, "scheduledend"),
        state: formatted(row, "statecode").or_else(|| text(row, "statecode")),
        milestone_id,
        regarding_type_verified: regarding_type.is_some(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MilestoneTasks {
    pub milestone_id: String,
    /// `None` when ownership could not be established or the result was truncated.
    pub has_user_task: Option<bool>,
    pub tasks: Vec<OwnedTask>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskCoverage {
    pub user: Option<SystemUser>,
    pub milestones: Vec<MilestoneTasks>,
    pub sources: Vec<Source>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AccountContext {
    pub account_id: String,
    pub opportunities: Sourced<Vec<Opportunity>>,
    pub milestones: Sourced<Vec<Milestone>>,
    pub tasks: TaskCoverage,
}

// ---------- reads ----------

fn query_key(tool: &mcp::ToolInfo) -> Option<&'static str> {
    let key = ["querytext", "query", "sql"]
        .into_iter()
        .find(|key| tool.property(key).is_some())?;
    tool.required()
        .iter()
        .all(|required| *required == key || *required == "top")
        .then_some(key)
}

fn describe_arguments(tool: &mcp::ToolInfo, table: &str) -> Option<Map<String, Value>> {
    let (key, value) = if tool.property("path").is_some() {
        ("path", format!("tables/{table}"))
    } else {
        let key = ["table", "tablename", "table_name", "entity", "logicalname"]
            .into_iter()
            .find(|key| tool.property(key).is_some())?;
        (key, table.to_owned())
    };
    tool.required()
        .iter()
        .all(|required| *required == key)
        .then(|| Map::from_iter([(key.to_owned(), Value::String(value))]))
}

fn guid_literal(id: &str) -> Result<String, String> {
    let id = id
        .trim()
        .trim_matches(|character| character == '{' || character == '}');
    if !super::auth::uuid(id) {
        return Err("Expected an MSX record ID (GUID)".into());
    }
    Ok(sql_string(&id.to_ascii_lowercase()))
}

impl Client {
    /// Runs one bounded `read_query` and returns at most `top` rows with their source.
    pub(super) async fn read(
        &self,
        table: &str,
        sql: String,
        top: usize,
    ) -> Result<(Vec<Value>, Source), String> {
        let connection = self
            .connection(Resource::Dataverse)
            .await
            .map_err(|error| error.message().to_owned())?;
        let tool = connection
            .tools
            .get("read_query")
            .ok_or("Dataverse does not offer read_query")?;
        let key = query_key(tool).ok_or("Dataverse read_query has an unsupported schema")?;
        let mut arguments = Map::from_iter([(key.to_owned(), Value::String(sql.clone()))]);
        if tool.property("top").is_some() {
            arguments.insert("top".into(), json!(top));
        }
        let value = self
            .call(Resource::Dataverse, "read_query", arguments, table == "ai")
            .await?;
        let rows = mcp::rows(&value).ok_or("Dataverse returned rows in an unsupported format")?;
        let truncated = rows.len() >= top;
        let rows = rows.iter().take(top).cloned().collect::<Vec<_>>();
        Ok((
            rows.clone(),
            Source {
                system: "Dataverse",
                table: table.into(),
                query: sql,
                retrieved_at: Utc::now(),
                rows: rows.len(),
                truncated,
            },
        ))
    }

    async fn describe_raw(&self, table: &str) -> Result<Value, String> {
        let connection = self
            .connection(Resource::Dataverse)
            .await
            .map_err(|error| error.message().to_owned())?;
        let (tool, arguments) = ["describe_table", "describe"]
            .into_iter()
            .find_map(|name| {
                let tool = connection.tools.get(name)?;
                Some((name, describe_arguments(tool, table)?))
            })
            .ok_or("Dataverse offers no supported schema description tool")?;
        self.call(Resource::Dataverse, tool, arguments, false).await
    }

    /// Column names of a table, discovered once per session; unknown when discovery fails.
    pub async fn table_schema(&self, table: &str) -> Arc<TableSchema> {
        if let Some(schema) = self
            .schemas
            .lock()
            .ok()
            .and_then(|schemas| schemas.get(table).cloned())
        {
            return schema;
        }
        let schema = Arc::new(
            self.describe_raw(table)
                .await
                .map(|value| TableSchema::parse(&value))
                .unwrap_or_default(),
        );
        if let Ok(mut schemas) = self.schemas.lock() {
            schemas.insert(table.into(), schema.clone());
        }
        schema
    }

    /// The signed-in user's Dataverse `systemuser`, resolved from the verified token identity.
    /// The cached user belongs to the token identity it was resolved for; another account's
    /// token resolves its own user.
    pub async fn whoami(&self) -> Result<Sourced<SystemUser>, String> {
        let claims = self
            .connection(Resource::Dataverse)
            .await
            .map_err(|error| error.message().to_owned())?
            .claims;
        let mut cached = self.user.lock().await;
        if let Some((identity, user)) = cached.as_ref()
            && *identity == claims
        {
            return Ok(user.clone());
        }
        *cached = None;
        let mut lookups = Vec::new();
        if let Some(object_id) = &claims.object_id {
            lookups.push((
                format!("azureactivedirectoryobjectid = {}", sql_string(object_id)),
                "Entra object ID",
            ));
        }
        if let Some(account) = &claims.account {
            lookups.push((
                format!(
                    "domainname = {0} OR internalemailaddress = {0}",
                    sql_string(account)
                ),
                "sign-in name",
            ));
        }
        if lookups.is_empty() {
            return Err("The Azure CLI token has no identity to resolve an MSX user".into());
        }
        for (filter, matched_by) in lookups {
            let sql = format!(
                "SELECT TOP 2 systemuserid, fullname, internalemailaddress FROM systemuser WHERE {filter}"
            );
            let (rows, source) = self.read("systemuser", sql, 2).await?;
            if let [row] = rows.as_slice()
                && let Some(id) = guid(row, "systemuserid")
            {
                let user = Sourced {
                    value: SystemUser {
                        id,
                        name: text(row, "fullname"),
                        email: text(row, "internalemailaddress"),
                        matched_by,
                    },
                    source,
                };
                *cached = Some((claims.clone(), user.clone()));
                return Ok(user);
            }
        }
        Err("The signed-in identity did not resolve to exactly one MSX user; owned-task checks are unavailable.".into())
    }

    /// Candidate customer accounts for meeting e-mail domains and company names, with
    /// confidence and evidence. Never selects silently when the match is ambiguous.
    pub async fn find_accounts(&self, query: &AccountQuery) -> Result<AccountLookup, String> {
        let (domains, names, rejected_inputs) = query.prepare();
        if domains.is_empty() && names.is_empty() {
            return Ok(AccountLookup {
                candidates: Vec::new(),
                selection: Selection::NoMatch,
                requires_confirmation: true,
                rejected_inputs,
                sources: Vec::new(),
            });
        }
        let schema = self.table_schema("account").await;
        let columns = schema.select(
            &["accountid", "name"],
            &["websiteurl", "accountnumber", "ownerid"],
            &[
                "owneridname",
                "msp_mstopparentid",
                "msp_tpid",
                "msp_accounttpid",
            ],
        );
        let mut sources = Vec::new();
        let mut accounts = BTreeMap::<String, AccountCandidate>::new();
        let mut contacts = BTreeMap::<String, BTreeMap<String, usize>>::new();
        let mut filters = Vec::new();
        if !domains.is_empty() {
            filters.push(
                domains
                    .iter()
                    .map(|domain| format!("websiteurl LIKE {}", like_contains(domain)))
                    .collect::<Vec<_>>()
                    .join(" OR "),
            );
        }
        if !names.is_empty() {
            filters.push(
                names
                    .iter()
                    .map(|name| format!("name LIKE {}", like_contains(name)))
                    .collect::<Vec<_>>()
                    .join(" OR "),
            );
        }
        for filter in filters {
            let sql = format!(
                "SELECT TOP 25 {} FROM account WHERE statecode = 0 AND ({filter})",
                columns.join(", ")
            );
            let (rows, source) = self.read("account", sql, 25).await?;
            sources.push(source);
            for candidate in rows.iter().filter_map(account) {
                accounts.entry(candidate.id.clone()).or_insert(candidate);
            }
        }
        let joined = columns
            .iter()
            .map(|column| format!("account.{column}"))
            .collect::<Vec<_>>()
            .join(", ");
        for domain in domains.iter().take(3) {
            let sql = format!(
                "SELECT TOP 50 {joined} FROM contact JOIN account ON contact.parentcustomerid = account.accountid WHERE contact.statecode = 0 AND account.statecode = 0 AND contact.emailaddress1 LIKE {}",
                sql_string(&format!("%@{domain}"))
            );
            let (rows, source) = self.read("contact, account", sql, 50).await?;
            sources.push(source);
            for candidate in rows.iter().filter_map(account) {
                *contacts
                    .entry(candidate.id.clone())
                    .or_default()
                    .entry(domain.clone())
                    .or_default() += 1;
                accounts.entry(candidate.id.clone()).or_insert(candidate);
            }
        }
        let mut candidates = accounts
            .into_values()
            .map(|mut candidate| {
                let hits = contacts.get(&candidate.id).cloned().unwrap_or_default();
                score(&mut candidate, &domains, &names, &hits);
                candidate
            })
            .filter(|candidate| !candidate.evidence.is_empty())
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .confidence
                .total_cmp(&left.confidence)
                .then_with(|| left.name.cmp(&right.name))
        });
        candidates.truncate(10);
        Ok(AccountLookup {
            selection: select(&candidates),
            candidates,
            requires_confirmation: true,
            rejected_inputs,
            sources,
        })
    }

    /// Open opportunities, active future milestones and the user's owned milestone tasks.
    pub async fn account_context(
        &self,
        account_id: &str,
        from: NaiveDate,
        until: NaiveDate,
    ) -> Result<AccountContext, String> {
        let account = guid_literal(account_id)?;
        if until < from || (until - from).num_days() > 366 {
            return Err("Milestone window must cover 0 to 366 days".into());
        }
        let opportunity_schema = self.table_schema("opportunity").await;
        let account_column = if opportunity_schema.known()
            && !opportunity_schema.has("parentaccountid")
            && opportunity_schema.has("customerid")
        {
            "customerid"
        } else {
            "parentaccountid"
        };
        let columns = opportunity_schema.select(
            &["opportunityid", "name"],
            &[
                "stepname",
                "estimatedclosedate",
                "estimatedvalue",
                "ownerid",
            ],
            &["owneridname", "msp_salesstagecode"],
        );
        let sql = format!(
            "SELECT TOP 50 {} FROM opportunity WHERE {account_column} = {account} AND statecode = 0 ORDER BY estimatedclosedate",
            columns.join(", ")
        );
        let (rows, source) = self.read("opportunity", sql, 50).await?;
        let opportunities = Sourced {
            value: rows.iter().filter_map(opportunity).collect(),
            source,
        };
        let milestone_schema = self.table_schema("msp_engagementmilestone").await;
        let columns = milestone_schema
            .select(
                &["msp_engagementmilestoneid", "msp_name"],
                &[
                    "msp_milestonedate",
                    "msp_milestonestatus",
                    "msp_workload",
                    "msp_opportunityid",
                    "ownerid",
                ],
                &["msp_milestonestatusname", "msp_workloadname", "owneridname"],
            )
            .into_iter()
            .map(|column| format!("msp_engagementmilestone.{column}"))
            .collect::<Vec<_>>();
        let sql = format!(
            "SELECT TOP 100 {} FROM msp_engagementmilestone JOIN opportunity ON msp_engagementmilestone.msp_opportunityid = opportunity.opportunityid WHERE opportunity.{account_column} = {account} AND msp_engagementmilestone.statecode = 0 AND msp_engagementmilestone.msp_milestonedate >= {} AND msp_engagementmilestone.msp_milestonedate <= {} ORDER BY msp_engagementmilestone.msp_milestonedate",
            columns.join(", "),
            sql_string(&from.to_string()),
            sql_string(&until.to_string())
        );
        let (rows, source) = self.read("msp_engagementmilestone", sql, 100).await?;
        let mut seen = BTreeSet::new();
        let milestones = rows
            .iter()
            .filter_map(milestone)
            .filter(|milestone| open_milestone(milestone) && seen.insert(milestone.id.clone()))
            .collect::<Vec<_>>();
        let tasks = self.owned_tasks(&milestones).await;
        Ok(AccountContext {
            account_id: account.trim_matches('\'').to_owned(),
            opportunities,
            milestones: Sourced {
                value: milestones,
                source,
            },
            tasks,
        })
    }

    async fn owned_tasks(&self, milestones: &[Milestone]) -> TaskCoverage {
        let unknown = |note: String, user: Option<SystemUser>, sources: Vec<Source>| TaskCoverage {
            user,
            milestones: milestones
                .iter()
                .map(|milestone| MilestoneTasks {
                    milestone_id: milestone.id.clone(),
                    has_user_task: None,
                    tasks: Vec::new(),
                })
                .collect(),
            sources,
            note: Some(note),
        };
        let user = match self.whoami().await {
            Ok(user) => user,
            Err(error) => {
                return unknown(
                    format!("Owned-task check unavailable: {error}"),
                    None,
                    Vec::new(),
                );
            }
        };
        let mut sources = vec![user.source.clone()];
        let mut result = Vec::new();
        for batch in milestones.chunks(10) {
            let ids = batch
                .iter()
                .map(|milestone| milestone.id.clone())
                .collect::<BTreeSet<_>>();
            let regarding = ids
                .iter()
                .map(|id| format!("regardingobjectid = {}", sql_string(id)))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT TOP 50 activityid, subject, scheduledend, statecode, regardingobjectid, regardingobjecttypecode FROM task WHERE ownerid = {} AND ({regarding})",
                sql_string(&user.value.id)
            );
            let (rows, source) = match self.read("task", sql, 50).await {
                Ok(read) => read,
                Err(error) => {
                    return unknown(
                        format!("Owned-task check failed: {error}"),
                        Some(user.value),
                        sources,
                    );
                }
            };
            let truncated = source.truncated;
            sources.push(source);
            let tasks = rows
                .iter()
                .filter_map(|row| owned_task(row, &ids))
                .collect::<Vec<_>>();
            for milestone in batch {
                let owned = tasks
                    .iter()
                    .filter(|task| task.milestone_id == milestone.id)
                    .cloned()
                    .collect::<Vec<_>>();
                result.push(MilestoneTasks {
                    milestone_id: milestone.id.clone(),
                    has_user_task: if !owned.is_empty() {
                        Some(true)
                    } else if truncated {
                        None
                    } else {
                        Some(false)
                    },
                    tasks: owned,
                });
            }
        }
        TaskCoverage {
            user: Some(user.value),
            milestones: result,
            sources,
            note: None,
        }
    }

    /// `tomlook_msx_query`: a validated SELECT over the MSX allowlist, bounded to 50 rows.
    pub async fn ai_query(&self, sql: &str) -> Result<Value, String> {
        let (sql, tables) = validate_ai_query(sql)?;
        let (rows, mut source) = self.read("ai", sql, AI_ROW_LIMIT).await?;
        source.table = tables.join(", ");
        Ok(super::tools::bounded(
            "MSX Dataverse (read-only)",
            &source,
            rows,
        ))
    }

    /// `tomlook_msx_describe`: the bounded schema description of one allowlisted table.
    pub async fn ai_describe(&self, table: &str) -> Result<Value, String> {
        if !AI_TABLES.contains(&table) {
            return Err(format!(
                "Describe only these tables: {}",
                AI_TABLES.join(", ")
            ));
        }
        let value = self.describe_raw(table).await?;
        if let Ok(mut schemas) = self.schemas.lock() {
            schemas.insert(table.into(), Arc::new(TableSchema::parse(&value)));
        }
        let text = match value {
            Value::String(text) => text,
            value => serde_json::to_string(&value).map_err(|_| "Encode schema failed")?,
        };
        let truncated = text.len() > 30_000;
        let schema = &text[..text.floor_char_boundary(30_000)];
        Ok(json!({
            "source": "MSX Dataverse (read-only)",
            "untrusted_data": true,
            "table": table,
            "retrieved_at": Utc::now(),
            "schema": schema,
            "truncated": truncated,
        }))
    }
}

#[cfg(test)]
mod tests;
