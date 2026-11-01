//! Azure consumption revenue (ACR) from the Fabric/Power BI semantic model.
use super::{Client, Resource, mcp};
use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub const MODEL_ID: &str = "f7ecc250-c244-43a6-aea5-7a957f9e9d38";
pub const MODEL_NAME: &str = "AzureBlueSubscriptionSL4";
pub const MEASURE: &str = "$ ACR";
pub const DAILY_MEASURE: &str = "$ Average Daily ACR";
pub const UNIT: &str = "USD";
pub const AI_ROW_LIMIT: usize = 200;
const NO_PREDICTION: &str = "Prediction omitted: the model exposes no calibrated forecast measure; the current month is a partial actual.";

/// Which elements of the semantic model this account can use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelSchema {
    /// Read from the live model, not assumed from the documented names.
    pub verified: bool,
    pub acr: bool,
    pub daily: bool,
    pub customer_name: bool,
    pub tpid: bool,
    pub date: bool,
    pub forecast: Option<String>,
}

fn standalone(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(index, _)| {
        let before = text[..index].chars().next_back();
        let after = text[index + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn names(value: &Value, found: &mut Vec<String>, depth: usize) {
    if depth > 16 || found.len() > 5000 {
        return;
    }
    match value {
        Value::String(text) => {
            for part in text.split('[').skip(1) {
                if let Some((name, _)) = part.split_once(']') {
                    found.push(name.to_owned());
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| names(item, found, depth + 1)),
        Value::Object(object) => {
            for (key, item) in object {
                match (key.eq_ignore_ascii_case("name"), item.as_str()) {
                    (true, Some(name)) => found.push(name.to_owned()),
                    _ => names(item, found, depth + 1),
                }
            }
        }
        _ => {}
    }
}

impl ModelSchema {
    /// The names documented for AzureBlueSubscriptionSL4, used when schema discovery is not
    /// available; the query itself then verifies them.
    pub fn documented() -> Self {
        Self {
            verified: false,
            acr: true,
            daily: true,
            customer_name: true,
            tpid: false,
            date: true,
            forecast: None,
        }
    }

    pub fn detect(value: &Value) -> Self {
        let text = match value {
            Value::String(text) => text.clone(),
            value => serde_json::to_string(value).unwrap_or_default(),
        };
        let mut found = Vec::new();
        names(value, &mut found, 0);
        let forecast = found
            .into_iter()
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                lower.contains("acr")
                    && ["forecast", "predict", "projection", "projected"]
                        .iter()
                        .any(|marker| lower.contains(marker))
                    && name.len() <= 100
                    && !name
                        .chars()
                        .any(|character| character.is_control() || "[]\"'".contains(character))
            })
            .min();
        Self {
            verified: true,
            acr: text.contains(MEASURE),
            daily: text.contains(DAILY_MEASURE),
            customer_name: text.contains("DimCustomer") && text.contains("TPAccountName"),
            tpid: text.contains("DimCustomer") && standalone(&text, "TPID"),
            date: text.contains("DimDate") && text.contains("DateID"),
            forecast,
        }
    }

    pub fn usable(&self) -> bool {
        self.acr && self.date && (self.customer_name || self.tpid)
    }
}

/// How Power BI's query tool takes its model ID and DAX text, read from its live schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryShape {
    model: String,
    query: Option<(String, bool)>,
    limit: Option<String>,
}

impl QueryShape {
    pub fn from_tool(tool: &mcp::ToolInfo, needs_query: bool) -> Result<Self, String> {
        let properties = tool
            .schema
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("Power BI tool has no argument schema")?;
        let keys = properties.keys().map(String::as_str).collect::<Vec<_>>();
        let model = ["semanticModelId", "datasetId", "artifactId", "modelId"]
            .into_iter()
            .find(|key| keys.contains(key))
            .or_else(|| {
                keys.iter().copied().find(|key| {
                    let lower = key.to_ascii_lowercase();
                    lower.ends_with("id")
                        && ["model", "dataset", "artifact"]
                            .iter()
                            .any(|kind| lower.contains(kind))
                })
            })
            .ok_or("Power BI tool has no semantic model argument")?
            .to_owned();
        let query = keys
            .iter()
            .copied()
            .find(|key| {
                let lower = key.to_ascii_lowercase();
                lower.contains("dax") || lower == "query" || lower == "queries"
            })
            .map(|key| {
                let array = properties[key].get("type").and_then(Value::as_str) == Some("array");
                (key.to_owned(), array)
            });
        if needs_query && query.is_none() {
            return Err("Power BI query tool has no DAX argument".into());
        }
        let limit = keys
            .iter()
            .copied()
            .find(|key| {
                let lower = key.to_ascii_lowercase();
                (lower.contains("maxrows") || lower.contains("rowlimit") || lower == "top")
                    && properties[*key].get("type").and_then(Value::as_str) == Some("integer")
            })
            .map(str::to_owned);
        let known = [
            Some(model.as_str()),
            query.as_ref().map(|(key, _)| key.as_str()),
            limit.as_deref(),
        ];
        if tool
            .required()
            .iter()
            .any(|required| !known.contains(&Some(*required)))
        {
            return Err("Power BI tool requires unsupported arguments".into());
        }
        Ok(Self {
            model,
            query,
            limit,
        })
    }

    pub fn arguments(&self, dax: Option<&str>, rows: usize) -> Map<String, Value> {
        let mut arguments = Map::from_iter([(self.model.clone(), Value::String(MODEL_ID.into()))]);
        if let (Some((key, array)), Some(dax)) = (&self.query, dax) {
            arguments.insert(key.clone(), if *array { json!([dax]) } else { json!(dax) });
        }
        if let Some(key) = &self.limit {
            arguments.insert(key.clone(), json!(rows));
        }
        arguments
    }
}

/// A validated customer key: digits-only TPID and/or a bounded customer name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Customer {
    pub tpid: Option<String>,
    pub name: Option<String>,
}

impl Customer {
    pub fn new(tpid: Option<&str>, name: Option<&str>) -> Result<Self, String> {
        let tpid = tpid.map(str::trim).filter(|tpid| !tpid.is_empty());
        if tpid
            .is_some_and(|tpid| tpid.len() > 20 || !tpid.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Err("A TPID contains only digits".into());
        }
        let name = name.map(str::trim).filter(|name| !name.is_empty());
        if name.is_some_and(|name| name.chars().count() > 200 || name.chars().any(char::is_control))
        {
            return Err("Customer name is too long or contains control characters".into());
        }
        if tpid.is_none() && name.is_none() {
            return Err("ACR needs a TPID or a customer name".into());
        }
        Ok(Self {
            tpid: tpid.map(str::to_owned),
            name: name.map(str::to_owned),
        })
    }
}

fn dax_string(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// Monthly ACR for one customer, newest months first. TPID comparison is text-based so it works
/// whether the column is numeric or text.
pub fn monthly_query(
    customer: &Customer,
    schema: &ModelSchema,
    months: u8,
) -> Option<(String, &'static str)> {
    let (filter, matched_by) = match (&customer.tpid, &customer.name) {
        (Some(tpid), _) if schema.tpid => (
            format!(
                "FILTER(ALL('DimCustomer'[TPID]), 'DimCustomer'[TPID] & \"\" = {})",
                dax_string(tpid)
            ),
            "TPID",
        ),
        (_, Some(name)) if schema.customer_name => (
            format!(
                "FILTER(ALL('DimCustomer'[TPAccountName]), 'DimCustomer'[TPAccountName] = {})",
                dax_string(name)
            ),
            "customer name",
        ),
        _ => return None,
    };
    let mut measures = format!("\"ACR\", [{MEASURE}]");
    if schema.daily {
        measures.push_str(&format!(", \"AverageDaily\", [{DAILY_MEASURE}]"));
    }
    if let Some(forecast) = &schema.forecast {
        measures.push_str(&format!(", \"Forecast\", [{forecast}]"));
    }
    Some((
        format!(
            "EVALUATE\nTOPN({}, SUMMARIZECOLUMNS('DimDate'[DateID], 'DimDate'[FiscalYear], 'DimDate'[FiscalMonth], {filter}, {measures}), 'DimDate'[DateID], DESC)\nORDER BY 'DimDate'[DateID] DESC",
            u32::from(months) + 4
        ),
        matched_by,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A completed month.
    Actual,
    /// The current month so far.
    PartialActual,
    /// A model forecast; never Tomlook's extrapolation.
    Prediction,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Month {
    /// First day of the calendar month.
    pub month: NaiveDate,
    pub label: String,
    pub fiscal: Option<String>,
    pub value: f64,
    pub kind: Kind,
    pub average_daily: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Series {
    pub customer: Customer,
    pub matched_by: &'static str,
    pub model_id: &'static str,
    pub model_name: &'static str,
    pub measure: &'static str,
    pub unit: &'static str,
    pub months: Vec<Month>,
    /// Freshness: the newest month with data. The model's refresh time is not exposed.
    pub latest_month: Option<NaiveDate>,
    pub retrieved_at: DateTime<Utc>,
    pub prediction_note: String,
    pub query: String,
    pub note: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Acr {
    Available(Box<Series>),
    NoData { reason: String },
    Unavailable { reason: String },
}

fn cell<'a>(row: &'a Value, column: &str) -> Option<&'a Value> {
    let suffix = format!("[{column}]");
    row.as_object()?
        .iter()
        .find(|(key, _)| key.ends_with(&suffix) || key.as_str() == column)
        .map(|(_, value)| value)
        .filter(|value| !value.is_null())
}

fn cell_text(row: &Value, column: &str) -> Option<String> {
    match cell(row, column)? {
        Value::String(text) => Some(text.trim().to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn cell_number(row: &Value, column: &str) -> Option<f64> {
    match cell(row, column)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn first_of(year: i32, month: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, 1).filter(|_| (2000..=2100).contains(&year))
}

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Calendar month of a result row from its monthly `DateID`, or from fiscal year and month
/// (Microsoft's fiscal year starts on July 1).
pub fn row_month(row: &Value) -> Option<NaiveDate> {
    if let Some(id) = cell_text(row, "DateID") {
        let digits = id.bytes().all(|byte| byte.is_ascii_digit());
        let parsed = if digits && (id.len() == 8 || id.len() == 6) {
            first_of(id[..4].parse().ok()?, id[4..6].parse().ok()?)
        } else if id.len() >= 7 && id.as_bytes()[4] == b'-' {
            first_of(id[..4].parse().ok()?, id[5..7].parse().ok()?)
        } else if let Some(date) = id
            .split_whitespace()
            .next()
            .and_then(|date| NaiveDate::parse_from_str(date, "%m/%d/%Y").ok())
        {
            first_of(date.year(), date.month())
        } else {
            None
        };
        if parsed.is_some() {
            return parsed;
        }
    }
    let year = cell_text(row, "FiscalYear")?;
    let year = year.trim_start_matches(|character: char| !character.is_ascii_digit());
    let year: i32 = match year.parse::<i32>().ok()? {
        year @ 0..=99 => 2000 + year,
        year => year,
    };
    let month = cell_text(row, "FiscalMonth")?.to_ascii_lowercase();
    if let Some(calendar) = MONTH_NAMES
        .iter()
        .position(|name| month.starts_with(name))
        .map(|index| index as u32 + 1)
    {
        return first_of(if calendar >= 7 { year - 1 } else { year }, calendar);
    }
    let fiscal: u32 = month
        .trim_start_matches(|character: char| !character.is_ascii_digit())
        .parse()
        .ok()
        .filter(|month| (1..=12).contains(month))?;
    let calendar = (fiscal + 5) % 12 + 1;
    first_of(if fiscal <= 6 { year - 1 } else { year }, calendar)
}

/// Labels completed, current partial and (model-forecast) predicted months, oldest first.
pub fn label_months(rows: &[Value], today: NaiveDate, months: u8) -> Result<Vec<Month>, String> {
    let current = first_of(today.year(), today.month()).ok_or("Invalid current date")?;
    let oldest = current
        .checked_sub_months(Months::new(u32::from(months)))
        .ok_or("Invalid ACR window")?;
    let mut result = Vec::new();
    let mut unreadable = 0;
    for row in rows {
        let Some(month) = row_month(row) else {
            unreadable += 1;
            continue;
        };
        if month < oldest
            || result
                .iter()
                .any(|known: &Month| known.month == month && known.kind != Kind::Prediction)
        {
            continue;
        }
        let fiscal = match (cell_text(row, "FiscalYear"), cell_text(row, "FiscalMonth")) {
            (Some(year), Some(month)) => Some(format!("{year} {month}")),
            _ => None,
        };
        let label = month.format("%Y-%m").to_string();
        let average_daily = cell_number(row, "AverageDaily");
        if let Some(value) = cell_number(row, "ACR").filter(|_| month <= current) {
            result.push(Month {
                month,
                label: label.clone(),
                fiscal: fiscal.clone(),
                value,
                kind: if month < current {
                    Kind::Actual
                } else {
                    Kind::PartialActual
                },
                average_daily,
            });
        }
        if let Some(value) = cell_number(row, "Forecast").filter(|_| month >= current) {
            result.push(Month {
                month,
                label,
                fiscal,
                value,
                kind: Kind::Prediction,
                average_daily: None,
            });
        }
    }
    if result.is_empty() && unreadable > 0 {
        return Err("ACR months could not be interpreted from the model's date fields".into());
    }
    result.sort_by(|left, right| {
        left.month
            .cmp(&right.month)
            .then((left.kind as u8).cmp(&(right.kind as u8)))
    });
    Ok(result)
}

impl Client {
    async fn model_schema(&self) -> Arc<ModelSchema> {
        let mut cached = self.model.lock().await;
        if let Some(schema) = cached.as_ref() {
            return schema.clone();
        }
        let detected = async {
            let connection = self
                .connection(Resource::PowerBi)
                .await
                .map_err(|error| error.message().to_owned())?;
            let tool = connection
                .tools
                .get("GetSemanticModelSchema")
                .ok_or("No schema tool")?;
            let shape = QueryShape::from_tool(tool, false)?;
            let value = self
                .call(
                    Resource::PowerBi,
                    "GetSemanticModelSchema",
                    shape.arguments(None, 0),
                    false,
                )
                .await?;
            Ok::<_, String>(ModelSchema::detect(&value))
        }
        .await;
        let schema = Arc::new(detected.unwrap_or_else(|_| ModelSchema::documented()));
        *cached = Some(schema.clone());
        schema
    }

    async fn execute(
        &self,
        dax: &str,
        limit: usize,
        detail: bool,
    ) -> Result<(Vec<Value>, bool), String> {
        let connection = self
            .connection(Resource::PowerBi)
            .await
            .map_err(|error| error.message().to_owned())?;
        let tool = connection
            .tools
            .get("ExecuteQuery")
            .ok_or("Power BI does not offer ExecuteQuery")?;
        let shape = QueryShape::from_tool(tool, true)?;
        let value = self
            .call(
                Resource::PowerBi,
                "ExecuteQuery",
                shape.arguments(Some(dax), limit),
                detail,
            )
            .await?;
        let rows = mcp::rows(&value).ok_or("Power BI returned rows in an unsupported format")?;
        Ok((
            rows.iter().take(limit).cloned().collect(),
            rows.len() > limit,
        ))
    }

    /// Monthly ACR for the last `months` (6 to 12) completed months plus the current partial
    /// month, labelled with source, unit and freshness. Unavailable states are explicit.
    pub async fn acr(&self, customer: &Customer, months: u8, today: NaiveDate) -> Acr {
        if !(6..=12).contains(&months) {
            return Acr::Unavailable {
                reason: "ACR history covers 6 to 12 months".into(),
            };
        }
        if !self.ready(Resource::PowerBi) && self.connection(Resource::PowerBi).await.is_err() {
            return Acr::Unavailable {
                reason: self.status().powerbi.message,
            };
        }
        let schema = self.model_schema().await;
        if !schema.usable() {
            return Acr::Unavailable {
                reason: format!(
                    "{MODEL_NAME} does not expose [{MEASURE}] with customer and date fields to this account."
                ),
            };
        }
        let Some((query, matched_by)) = monthly_query(customer, &schema, months) else {
            return Acr::Unavailable {
                reason: format!(
                    "{MODEL_NAME} has no TPID column for this customer key; use the customer name."
                ),
            };
        };
        let rows = match self.execute(&query, 64, false).await {
            Ok((rows, _)) => rows,
            Err(error) => return Acr::Unavailable { reason: error },
        };
        let months = match label_months(&rows, today, months) {
            Ok(months) => months,
            Err(reason) => return Acr::Unavailable { reason },
        };
        if months.is_empty() {
            return Acr::NoData {
                reason: format!(
                    "No ACR rows matched this customer by {matched_by} in {MODEL_NAME}."
                ),
            };
        }
        Acr::Available(Box::new(Series {
            customer: customer.clone(),
            matched_by,
            model_id: MODEL_ID,
            model_name: MODEL_NAME,
            measure: MEASURE,
            unit: UNIT,
            latest_month: months
                .iter()
                .filter(|month| month.kind != Kind::Prediction)
                .map(|month| month.month)
                .max(),
            months,
            retrieved_at: Utc::now(),
            prediction_note: match &schema.forecast {
                Some(forecast) => format!(
                    "Prediction from the model measure [{forecast}]; not extrapolated by Tomlook."
                ),
                None => NO_PREDICTION.into(),
            },
            query,
            note: "Consumption is technical evidence, not business intent, and never authorizes an MSX change.",
        }))
    }

    /// `tomlook_acr_query`: one DAX `EVALUATE` against the fixed ACR model, bounded to 200 rows.
    pub async fn ai_dax(&self, dax: &str) -> Result<Value, String> {
        let dax = validate_dax(dax)?;
        let (rows, truncated) = self.execute(&dax, AI_ROW_LIMIT, true).await?;
        let source = super::Source {
            system: "Power BI",
            table: format!("{MODEL_NAME} ({MODEL_ID})"),
            query: dax,
            retrieved_at: Utc::now(),
            rows: rows.len(),
            truncated,
        };
        Ok(super::tools::bounded(
            "Azure consumption, Power BI (read-only)",
            &source,
            rows,
        ))
    }
}

pub fn validate_dax(dax: &str) -> Result<String, String> {
    let dax = dax.trim();
    if dax.is_empty()
        || dax.len() > 8000
        || !dax
            .get(..8)
            .is_some_and(|start| start.eq_ignore_ascii_case("evaluate"))
        || !dax.chars().nth(8).is_some_and(char::is_whitespace)
        || dax
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err("Write one DAX query that starts with EVALUATE (at most 8,000 bytes)".into());
    }
    Ok(dax.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn months_are_labelled_completed_partial_or_model_predicted() {
        let rows = vec![
            json!({"DimDate[DateID]": 20261101, "DimDate[FiscalYear]": "FY27", "DimDate[FiscalMonth]": "M05", "[ACR]": 120.5, "[AverageDaily]": 60.25, "[Forecast]": 3600.0}),
            json!({"DimDate[DateID]": "2026-10-01T00:00:00", "[ACR]": 3100.0}),
            json!({"DimDate[DateID]": 202609, "[ACR]": "2900.5"}),
            json!({"DimDate[DateID]": 20251001, "[ACR]": 10.0}),
            json!({"DimDate[DateID]": 20261201, "[Forecast]": 3700.0}),
        ];
        let months = label_months(&rows, day(2026, 11, 2), 6).unwrap();
        let summary = months
            .iter()
            .map(|month| (month.label.as_str(), month.kind, month.value))
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                ("2026-09", Kind::Actual, 2900.5),
                ("2026-10", Kind::Actual, 3100.0),
                ("2026-11", Kind::PartialActual, 120.5),
                ("2026-11", Kind::Prediction, 3600.0),
                ("2026-12", Kind::Prediction, 3700.0),
            ]
        );
        assert_eq!(months[2].average_daily, Some(60.25));
        assert_eq!(months[2].fiscal.as_deref(), Some("FY27 M05"));
        assert_eq!(
            serde_json::to_value(Kind::PartialActual).unwrap(),
            "partial_actual"
        );
        assert!(label_months(&[json!({"[ACR]": 1.0})], day(2026, 11, 2), 6).is_err());
        assert!(label_months(&[], day(2026, 11, 2), 6).unwrap().is_empty());
    }

    #[test]
    fn fiscal_months_follow_the_july_fiscal_year() {
        let row = |year: Value, month: Value| json!({"'DimDate'[FiscalYear]": year, "'DimDate'[FiscalMonth]": month});
        assert_eq!(
            row_month(&row(json!("FY27"), json!(1))),
            Some(day(2026, 7, 1))
        );
        assert_eq!(
            row_month(&row(json!("FY27"), json!("P06"))),
            Some(day(2026, 12, 1))
        );
        assert_eq!(
            row_month(&row(json!(2027), json!(7))),
            Some(day(2027, 1, 1))
        );
        assert_eq!(
            row_month(&row(json!("FY27"), json!("Oct"))),
            Some(day(2026, 10, 1))
        );
        assert_eq!(
            row_month(&row(json!("FY27"), json!("March"))),
            Some(day(2027, 3, 1))
        );
        assert_eq!(
            row_month(&json!({"[DateID]": "10/1/2026 12:00:00 AM"})),
            Some(day(2026, 10, 1))
        );
        assert_eq!(row_month(&row(json!("FY27"), json!(13))), None);
    }

    #[test]
    fn schema_detection_and_query_building_escape_customer_values() {
        let schema = ModelSchema::detect(&json!({
            "tables": [
                {"name": "DimCustomer", "columns": [{"name": "TPAccountName"}, {"name": "TPID"}]},
                {"name": "DimDate", "columns": [{"name": "DateID"}, {"name": "FiscalMonth"}]},
                {"name": "Measures", "measures": [{"name": "$ ACR"}, {"name": "$ Average Daily ACR"}, {"name": "$ ACR Forecast"}]}
            ]
        }));
        assert!(schema.usable() && schema.tpid && schema.daily);
        assert_eq!(schema.forecast.as_deref(), Some("$ ACR Forecast"));
        let customer = Customer::new(Some("12345"), Some("Contoso \"Quoted\" a.s.")).unwrap();
        let (query, by) = monthly_query(&customer, &schema, 6).unwrap();
        assert_eq!(by, "TPID");
        assert!(query.starts_with("EVALUATE") && query.contains("TOPN(10,"));
        assert!(query.contains("'DimCustomer'[TPID] & \"\" = \"12345\""));
        assert!(query.contains("[$ ACR Forecast]"));
        let by_name = ModelSchema {
            tpid: false,
            forecast: None,
            ..schema.clone()
        };
        let (query, by) = monthly_query(&customer, &by_name, 12).unwrap();
        assert_eq!(by, "customer name");
        assert!(
            query.contains("= \"Contoso \"\"Quoted\"\" a.s.\")"),
            "{query}"
        );
        let tpid_only = Customer::new(Some("1"), None).unwrap();
        assert!(monthly_query(&tpid_only, &by_name, 6).is_none());
        assert!(Customer::new(Some("12'3"), None).is_err());
        assert!(Customer::new(None, None).is_err());
        assert!(!ModelSchema::detect(&json!("DimCustomer TPIDName")).tpid);
        assert!(!ModelSchema::default().usable());
        assert!(ModelSchema::documented().usable());
    }

    #[test]
    fn execute_query_arguments_follow_the_live_schema_and_reject_unknown_requirements() {
        let tool = mcp::ToolInfo::read(
            "ExecuteQuery",
            json!({
                "type":"object",
                "properties":{"artifactId":{"type":"string"},"daxQueries":{"type":"array","items":{"type":"string"}},"maxRows":{"type":"integer"}},
                "required":["artifactId","daxQueries"]
            }),
        );
        let shape = QueryShape::from_tool(&tool, true).unwrap();
        assert_eq!(
            Value::Object(shape.arguments(Some("EVALUATE ROW(\"x\", 1)"), 200)),
            json!({"artifactId": MODEL_ID, "daxQueries": ["EVALUATE ROW(\"x\", 1)"], "maxRows": 200})
        );
        let plain = mcp::ToolInfo::read(
            "ExecuteQuery",
            json!({
                "type":"object","properties":{"semanticModelId":{"type":"string"},"daxQuery":{"type":"string"}},
                "required":["semanticModelId","daxQuery"]
            }),
        );
        assert_eq!(
            Value::Object(
                QueryShape::from_tool(&plain, true)
                    .unwrap()
                    .arguments(Some("EVALUATE x"), 5)
            ),
            json!({"semanticModelId": MODEL_ID, "daxQuery": "EVALUATE x"})
        );
        let unknown = mcp::ToolInfo::read(
            "ExecuteQuery",
            json!({
                "type":"object","properties":{"semanticModelId":{"type":"string"},"daxQuery":{"type":"string"},"workspace":{"type":"string"}},
                "required":["semanticModelId","daxQuery","workspace"]
            }),
        );
        assert!(QueryShape::from_tool(&unknown, true).is_err());
    }

    #[test]
    fn model_queries_must_be_a_single_evaluate() {
        assert!(validate_dax("EVALUATE ROW(\"x\", 1)").is_ok());
        assert!(validate_dax("  evaluate\n'DimDate'").is_ok());
        for invalid in [
            "",
            "DEFINE MEASURE x = 1 EVALUATE ROW(\"x\", [x])",
            "EVALUATEx",
            "EVALUATE \u{0}",
        ] {
            assert!(validate_dax(invalid).is_err(), "{invalid}");
        }
        assert!(validate_dax(&format!("EVALUATE {}", "x".repeat(8000))).is_err());
    }
}
