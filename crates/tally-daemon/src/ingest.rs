//! Newline-delimited JSON events into the store, with the file-complete watermark rule.
use crate::store::{AppendSummary, Store, StoreError};
use tally_scoring::Event;

#[derive(thiserror::Error, Debug)]
pub enum IngestError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("line {line}: {source}")]
    Parse { line: usize, source: serde_json::Error },
}

/// Parses every line first (blank lines skipped); writes nothing if any line fails.
/// The watermark for `source` is `watermark`, else the newest `ts` in `text`;
/// it is not set when neither exists.
pub fn ingest_ndjson(
    store: &mut Store,
    text: &str,
    source: &str,
    watermark: Option<i64>,
) -> Result<AppendSummary, IngestError> {
    let mut events = Vec::new();
    for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        events.push(serde_json::from_str::<Event>(line).map_err(|e| IngestError::Parse { line: i + 1, source: e })?);
    }
    let mark = watermark.or_else(|| events.iter().map(|e| e.ts).max());
    let summary = store.append_events(&events)?;
    if let Some(ms) = mark {
        store.set_watermark(source, ms)?;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tally_scoring::Kind;

    fn line(id: &str, ts: i64) -> String {
        let e = Event { v: 1, id: id.into(), ts, source: "t".into(), project: None, session: None, kind: Kind::Interrupt };
        serde_json::to_string(&e).unwrap()
    }

    #[test]
    fn bad_line_is_numbered_and_nothing_is_written() {
        let mut s = Store::open_in_memory().unwrap();
        let text = format!("{}\n\nnot json\n", line("1", 5));
        let e = ingest_ndjson(&mut s, &text, "x", None).unwrap_err();
        assert!(e.to_string().starts_with("line 3: "), "{e}");
        assert!(s.events().unwrap().is_empty());
        assert_eq!(s.horizon().unwrap(), None);
    }

    #[test]
    fn leading_utf8_bom_is_ignored() {
        let mut s = Store::open_in_memory().unwrap();
        let text = format!("\u{feff}{}\n", line("1", 5));
        assert_eq!(ingest_ndjson(&mut s, &text, "x", None).unwrap().added, 1);
    }

    #[test]
    fn default_watermark_is_newest_ts() {
        let mut s = Store::open_in_memory().unwrap();
        let text = format!("{}\n{}\n", line("1", 5), line("2", 9));
        ingest_ndjson(&mut s, &text, "x", None).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(9));
    }

    #[test]
    fn explicit_watermark_wins() {
        let mut s = Store::open_in_memory().unwrap();
        ingest_ndjson(&mut s, &line("1", 5), "x", Some(100)).unwrap();
        assert_eq!(s.horizon().unwrap(), Some(100));
    }

    #[test]
    fn empty_input_without_watermark_sets_none() {
        let mut s = Store::open_in_memory().unwrap();
        let r = ingest_ndjson(&mut s, "\n  \n", "x", None).unwrap();
        assert_eq!((r.added, r.duplicates), (0, 0));
        assert_eq!(s.horizon().unwrap(), None);
    }

    #[test]
    fn reingest_reports_duplicates() {
        let mut s = Store::open_in_memory().unwrap();
        let text = format!("{}\n{}\n", line("1", 5), line("2", 9));
        ingest_ndjson(&mut s, &text, "x", None).unwrap();
        let r = ingest_ndjson(&mut s, &text, "x", None).unwrap();
        assert_eq!((r.added, r.duplicates), (0, 2));
    }
}
