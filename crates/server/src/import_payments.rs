//! Import of what was actually paid (`payments`) from a CSV kept outside git, one row per charge:
//!
//! ```text
//! paid_on,provider,product,kind,agent,amount_usd,reference,note
//! 2026-10-28,claude,Claude Max 20x,subscription,claude,200.00,,
//! ```
//!
//! `paid_on` is `YYYY-MM-DD`; `kind` is `subscription` or `usage`; `agent`, `reference` and `note`
//! may be empty; `amount_usd` may carry `$` and thousands separators. A charge is identified by
//! (provider, paid_on, amount_usd, reference): importing it again changes nothing, and a changed
//! product, kind, agent or note on a known charge is applied. Nothing is ever deleted.

use std::fmt;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use sqlx::MySqlPool;

use crate::import_cursor::read_csv;

const COLUMNS: [&str; 8] = ["paid_on", "provider", "product", "kind", "agent", "amount_usd", "reference", "note"];
const KINDS: [&str; 2] = ["subscription", "usage"];

#[derive(Debug, PartialEq)]
pub struct PaymentRow {
    pub paid_on: NaiveDate,
    pub provider: String,
    pub product: String,
    pub kind: String,
    pub agent: Option<String>,
    /// Two decimals, as stored (`DECIMAL(10,2)`).
    pub amount_usd: String,
    pub reference: String,
    pub note: String,
}

#[derive(Debug, Default)]
pub struct PaymentImportSummary {
    rows: usize,
    inserted: usize,
    updated: usize,
    unchanged: usize,
    total_usd: f64,
}

impl fmt::Display for PaymentImportSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "imported {} payments (${:.2}): {} new, {} updated, {} already stored",
            self.rows, self.total_usd, self.inserted, self.updated, self.unchanged
        )
    }
}

pub async fn from_csv(pool: &MySqlPool, path: &Path) -> Result<PaymentImportSummary> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let rows = parse(&text).with_context(|| format!("parsing {}", path.display()))?;
    if rows.is_empty() {
        bail!("{} has no payments", path.display());
    }

    let mut summary = PaymentImportSummary { rows: rows.len(), ..Default::default() };
    let mut tx = pool.begin().await?;
    for row in &rows {
        let context = || format!("storing the {} payment of {} on {}", row.provider, row.amount_usd, row.paid_on);

        // Looked up first: the connection reports found rows, so an upsert's affected count
        // can't tell a new charge from an unchanged one.
        let stored: Option<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT CAST(product AS CHAR), CAST(kind AS CHAR), CAST(agent AS CHAR), CAST(note AS CHAR)
             FROM payments WHERE provider = ? AND paid_on = ? AND amount_usd = ? AND reference = ? FOR UPDATE",
        )
        .bind(&row.provider)
        .bind(row.paid_on)
        .bind(&row.amount_usd)
        .bind(&row.reference)
        .fetch_optional(&mut *tx)
        .await
        .with_context(context)?;

        match stored {
            None => {
                sqlx::query(
                    "INSERT INTO payments (paid_on, provider, product, kind, agent, amount_usd, reference, note)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(row.paid_on)
                .bind(&row.provider)
                .bind(&row.product)
                .bind(&row.kind)
                .bind(&row.agent)
                .bind(&row.amount_usd)
                .bind(&row.reference)
                .bind(&row.note)
                .execute(&mut *tx)
                .await
                .with_context(context)?;
                summary.inserted += 1;
            }
            Some(stored) if stored == (row.product.clone(), row.kind.clone(), row.agent.clone(), row.note.clone()) => summary.unchanged += 1,
            Some(_) => {
                sqlx::query(
                    "UPDATE payments SET product = ?, kind = ?, agent = ?, note = ?
                     WHERE provider = ? AND paid_on = ? AND amount_usd = ? AND reference = ?",
                )
                .bind(&row.product)
                .bind(&row.kind)
                .bind(&row.agent)
                .bind(&row.note)
                .bind(&row.provider)
                .bind(row.paid_on)
                .bind(&row.amount_usd)
                .bind(&row.reference)
                .execute(&mut *tx)
                .await
                .with_context(context)?;
                summary.updated += 1;
            }
        }
        summary.total_usd += row.amount_usd.parse::<f64>().unwrap_or(0.0);
    }
    tx.commit().await?;

    Ok(summary)
}

pub fn parse(text: &str) -> Result<Vec<PaymentRow>> {
    let mut records = read_csv(text.strip_prefix('\u{feff}').unwrap_or(text))?.into_iter();
    let header = records.next().context("the file is empty")?;
    let index: Vec<usize> = COLUMNS
        .iter()
        .map(|name| header.iter().position(|column| column.trim() == *name).with_context(|| format!("missing column `{name}`")))
        .collect::<Result<_>>()?;

    let mut rows = Vec::new();
    for (number, record) in records.enumerate() {
        // Line numbers count the header; quoted multi-line notes would shift them, which is fine
        // for a file this simple.
        let line = number + 2;
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let field = |column: usize| record.get(index[column]).map(|value| value.trim()).unwrap_or("");
        let row = row(field).with_context(|| format!("line {line}"))?;
        rows.push(row);
    }
    Ok(rows)
}

fn row<'a>(field: impl Fn(usize) -> &'a str) -> Result<PaymentRow> {
    let paid_on = NaiveDate::parse_from_str(field(0), "%Y-%m-%d").with_context(|| format!("paid_on `{}` is not YYYY-MM-DD", field(0)))?;
    let kind = field(3);
    if !KINDS.contains(&kind) {
        bail!("kind `{kind}` must be one of {}", KINDS.join(", "));
    }
    let amount = field(5).trim_start_matches('$').replace(',', "");
    let amount_usd = match amount.parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 && value < 1e8 => format!("{value:.2}"),
        _ => bail!("amount_usd `{}` is not a dollar amount", field(5)),
    };

    Ok(PaymentRow {
        paid_on,
        provider: text(field(1), "provider", 32, true)?,
        product: text(field(2), "product", 160, true)?,
        kind: kind.to_owned(),
        agent: Some(text(field(4), "agent", 32, false)?).filter(|agent| !agent.is_empty()),
        amount_usd,
        reference: text(field(6), "reference", 128, false)?,
        note: text(field(7), "note", 255, false)?,
    })
}

fn text(value: &str, name: &str, max: usize, required: bool) -> Result<String> {
    if required && value.is_empty() {
        bail!("{name} is empty");
    }
    if value.chars().count() > max {
        bail!("{name} is longer than {max} characters");
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "paid_on,provider,product,kind,agent,amount_usd,reference,note\n";

    #[test]
    fn reads_rows() {
        let rows = parse(&format!(
            "\u{feff}{HEADER}2026-10-28,claude,Claude Max 20x,subscription,claude,200,,plan inferred\n\
             2023-02-09,chatgpt,ChatGPT Plus,subscription,codex,$20.00,,\"listed as \"\"Invoice\"\"\"\n\
             \n\
             2021-07-01,openai-api,OpenAI API,usage,,\"1,000.5\",INV-0001,\n"
        ))
        .unwrap();

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].amount_usd, "200.00");
        assert_eq!(rows[0].agent.as_deref(), Some("claude"));
        assert_eq!(rows[1].note, "listed as \"Invoice\"");
        assert_eq!(rows[2].agent, None);
        assert_eq!(rows[2].amount_usd, "1000.50");
        assert_eq!(rows[2].reference, "INV-0001");
    }

    #[test]
    fn columns_can_come_in_any_order() {
        let rows = parse("note,amount_usd,agent,kind,product,provider,paid_on,reference\nx,8,codex,subscription,ChatGPT Go,chatgpt,2026-05-28,\n").unwrap();
        assert_eq!(rows[0].product, "ChatGPT Go");
        assert_eq!(rows[0].amount_usd, "8.00");
    }

    #[test]
    fn rejects_bad_rows_with_their_line() {
        let error = |csv: &str| format!("{:#}", parse(&format!("{HEADER}{csv}")).unwrap_err());
        assert!(error("10/28/2026,claude,Pro,subscription,,20,,\n").contains("line 2"));
        assert!(error("2026-10-28,claude,Pro,monthly,,20,,\n").contains("kind `monthly`"));
        assert!(error("2026-10-28,claude,Pro,subscription,,twenty,,\n").contains("amount_usd"));
        assert!(error("2026-10-28,,Pro,subscription,,20,,\n").contains("provider is empty"));
        assert!(format!("{:#}", parse("paid_on,provider\n").unwrap_err()).contains("missing column `product`"));
    }
}
