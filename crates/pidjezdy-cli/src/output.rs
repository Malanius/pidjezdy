use std::error::Error;
use std::io::Write;

use chrono::{DateTime, Local, Utc};
use clap::ValueEnum;
use pidjezdy_core::selection::SelectedDeparture;
use serde::Serialize;
use thiserror::Error;
use unicode_width::UnicodeWidthStr;

use crate::AppError;
use crate::departures::DepartureQuery;

pub(crate) const JSON_SCHEMA_VERSION: u32 = 2;

const LINE_STYLE: &str = "\x1b[1;36m";
const EMPHASIS_STYLE: &str = "\x1b[1m";
const DIM_STYLE: &str = "\x1b[2m";
const RESET_STYLE: &str = "\x1b[0m";

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("could not serialize JSON output")]
    Json(#[from] serde_json::Error),
    #[error("could not write command output")]
    Write(#[from] std::io::Error),
}

#[derive(Serialize)]
struct JsonOutput<'a> {
    schema_version: u32,
    generated_at: DateTime<Utc>,
    data_updated_at: DateTime<Utc>,
    stale: bool,
    departures: &'a [SelectedDeparture],
    cancelled: &'a [SelectedDeparture],
}

#[derive(Serialize)]
struct JsonErrorOutput {
    schema_version: u32,
    generated_at: DateTime<Utc>,
    error: JsonError,
}

#[derive(Serialize)]
struct JsonError {
    kind: &'static str,
    message: String,
    causes: Vec<String>,
}

/// Write selected departures in the requested presentation format.
///
/// # Errors
///
/// Returns an error when JSON serialization or writing to the output fails.
pub(crate) fn write_departures(
    output: &mut impl Write,
    format: OutputFormat,
    query: &DepartureQuery,
    styled_text: bool,
) -> Result<(), OutputError> {
    match format {
        OutputFormat::Text => {
            write_text(
                output,
                query.generated_at,
                query.data_updated_at,
                query.stale,
                &query.departures,
                &query.cancelled,
                styled_text,
            )?;
        }
        OutputFormat::Json => {
            let document = serde_json::to_string(&JsonOutput {
                schema_version: JSON_SCHEMA_VERSION,
                generated_at: query.generated_at,
                data_updated_at: query.data_updated_at,
                stale: query.stale,
                departures: &query.departures,
                cancelled: &query.cancelled,
            })?;
            writeln!(output, "{document}")?;
        }
    }
    Ok(())
}

pub(crate) fn write_json_error(
    output: &mut impl Write,
    generated_at: DateTime<Utc>,
    error: &AppError,
) -> Result<(), OutputError> {
    let document = serde_json::to_string(&JsonErrorOutput {
        schema_version: JSON_SCHEMA_VERSION,
        generated_at,
        error: JsonError {
            kind: error.json_kind(),
            message: error.to_string(),
            causes: distinct_causes(error),
        },
    })?;
    writeln!(output, "{document}")?;
    Ok(())
}

/// Collect source messages that are not already included by their wrapper.
#[must_use]
pub fn distinct_causes(error: &(dyn Error + 'static)) -> Vec<String> {
    let mut causes = Vec::new();
    let mut previous = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if !previous.ends_with(&message) {
            causes.push(message.clone());
        }
        previous = message;
        source = cause.source();
    }
    causes
}

fn write_text(
    output: &mut impl Write,
    generated_at: DateTime<Utc>,
    data_updated_at: DateTime<Utc>,
    stale: bool,
    departures: &[SelectedDeparture],
    cancelled: &[SelectedDeparture],
    styled: bool,
) -> Result<(), std::io::Error> {
    if stale {
        let age_seconds = (generated_at - data_updated_at).num_seconds().max(0);
        writeln!(
            output,
            "STALE · last updated {} · {}",
            local_clock(data_updated_at),
            stale_age_label(age_seconds)
        )?;
    }
    if departures.is_empty() {
        writeln!(output, "No reachable departures.")?;
        if cancelled.is_empty() {
            return Ok(());
        }
    }

    let rows = departures.iter().map(TextRow::from).collect::<Vec<_>>();
    let line_width = rows.iter().map(|row| row.line.width()).max().unwrap_or(0);
    let detail_width = rows
        .iter()
        .flat_map(|row| [row.headsign.width(), row.boarding.width()])
        .max()
        .unwrap_or(0);
    let time_width = rows
        .iter()
        .flat_map(|row| [row.leave.width(), row.departs.width()])
        .max()
        .unwrap_or(0);
    let cancellation_notes = cancelled.iter().map(cancellation_note).collect::<Vec<_>>();
    let departure_width = line_width + detail_width + time_width + 4;
    let separator = "─".repeat(departure_width);

    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            write_styled_line(output, styled, DIM_STYLE, &separator)?;
        }

        write_styled_padded(
            output,
            styled,
            LINE_STYLE,
            row.line,
            line_width,
            Alignment::Left,
        )?;
        write!(output, "  ")?;
        write_styled_padded(
            output,
            styled,
            EMPHASIS_STYLE,
            row.headsign,
            detail_width,
            Alignment::Left,
        )?;
        write!(output, "  ")?;
        write_styled_padded(
            output,
            styled,
            EMPHASIS_STYLE,
            &row.leave,
            time_width,
            Alignment::Right,
        )?;
        writeln!(output)?;

        write_style(output, styled, DIM_STYLE)?;
        write_padding(output, line_width)?;
        write!(output, "  ")?;
        write_padded(output, &row.boarding, detail_width, Alignment::Left)?;
        write!(output, "  ")?;
        write_padded(output, &row.departs, time_width, Alignment::Right)?;
        write_style(output, styled, RESET_STYLE)?;
        writeln!(output)?;
    }

    if !rows.is_empty() && !cancellation_notes.is_empty() {
        write_styled_line(output, styled, DIM_STYLE, &separator)?;
    }
    for note in cancellation_notes {
        write_styled_line(output, styled, EMPHASIS_STYLE, &note)?;
    }
    Ok(())
}

fn stale_age_label(age_seconds: i64) -> String {
    if age_seconds < 60 {
        "just now".to_owned()
    } else if age_seconds < 3600 {
        format!("{} min ago", age_seconds / 60)
    } else if age_seconds < 86400 {
        format!("{}h {}m ago", age_seconds / 3600, age_seconds % 3600 / 60)
    } else {
        format!("{}d ago", age_seconds / 86400)
    }
}

fn cancellation_note(selected: &SelectedDeparture) -> String {
    let departure = &selected.departure;
    format!(
        "cancelled: {} → {} from {}, {}",
        departure.line,
        departure.headsign,
        selected.boarding_point_name,
        local_clock(departure.effective_at())
    )
}

fn local_clock(timestamp: DateTime<Utc>) -> String {
    timestamp.with_timezone(&Local).format("%H:%M").to_string()
}

fn delay_label(delay_seconds: Option<i64>) -> Option<String> {
    let delay_seconds = delay_seconds?;
    if delay_seconds >= 60 {
        Some(format!("+{} late", delay_seconds / 60))
    } else if delay_seconds <= -60 {
        Some(format!("-{} early", delay_seconds.unsigned_abs() / 60))
    } else {
        None
    }
}

struct TextRow<'a> {
    line: &'a str,
    headsign: &'a str,
    boarding: String,
    leave: String,
    departs: String,
}

impl<'a> From<&'a SelectedDeparture> for TextRow<'a> {
    fn from(selected: &'a SelectedDeparture) -> Self {
        let departure = &selected.departure;
        let platform = departure
            .platform_code
            .as_deref()
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .map_or_else(String::new, |code| format!(" · {code}"));
        let leave = if selected.leave_in_minutes() == 0 {
            "leave now".to_owned()
        } else {
            format!("leave in {} min", selected.leave_in_minutes())
        };

        let departs = format!("departs {}", local_clock(departure.effective_at()));
        let departs = match delay_label(departure.delay_seconds) {
            Some(delay) => format!("{departs} · {delay}"),
            None => departs,
        };

        Self {
            line: &departure.line,
            headsign: &departure.headsign,
            boarding: format!("{}{platform}", selected.boarding_point_name),
            leave,
            departs,
        }
    }
}

#[derive(Clone, Copy)]
enum Alignment {
    Left,
    Right,
}

fn write_styled_padded(
    output: &mut impl Write,
    styled: bool,
    style: &str,
    value: &str,
    width: usize,
    alignment: Alignment,
) -> Result<(), std::io::Error> {
    write_style(output, styled, style)?;
    write_padded(output, value, width, alignment)?;
    write_style(output, styled, RESET_STYLE)
}

fn write_padded(
    output: &mut impl Write,
    value: &str,
    width: usize,
    alignment: Alignment,
) -> Result<(), std::io::Error> {
    let padding = width.saturating_sub(value.width());
    if matches!(alignment, Alignment::Right) {
        write_padding(output, padding)?;
    }
    write!(output, "{value}")?;
    if matches!(alignment, Alignment::Left) {
        write_padding(output, padding)?;
    }
    Ok(())
}

fn write_padding(output: &mut impl Write, width: usize) -> Result<(), std::io::Error> {
    write!(output, "{:width$}", "")
}

fn write_style(output: &mut impl Write, styled: bool, style: &str) -> Result<(), std::io::Error> {
    if styled {
        write!(output, "{style}")?;
    }
    Ok(())
}

fn write_styled_line(
    output: &mut impl Write,
    styled: bool,
    style: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    if styled {
        writeln!(output, "{style}{value}{RESET_STYLE}")
    } else {
        writeln!(output, "{value}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use pidjezdy_core::departure::{Departure, Vehicle};

    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-04T12:00:00+02:00")
            .unwrap()
            .to_utc()
    }

    fn selected() -> SelectedDeparture {
        SelectedDeparture {
            departure: Departure {
                trip_id: "trip-1".into(),
                line: "158".into(),
                headsign: "Centre".into(),
                stop_id: "U100Z1P".into(),
                platform_code: Some(" A ".into()),
                scheduled_at: now() + TimeDelta::minutes(10),
                predicted_at: None,
                delay_seconds: None,
                is_cancelled: false,
                vehicle: Vehicle::default(),
            },
            boarding_point_name: "Nearby stop".into(),
            walking_minutes: 4,
            safety_buffer_minutes: 2,
            departs_in_seconds: 10 * 60 + 30,
            leave_in_seconds: 4 * 60 + 30,
            reachable: true,
        }
    }

    fn departure_query(
        departures: Vec<SelectedDeparture>,
        data_updated_at: DateTime<Utc>,
        stale: bool,
    ) -> DepartureQuery {
        DepartureQuery {
            generated_at: now(),
            data_updated_at,
            stale,
            departures,
            cancelled: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn text_output_uses_aligned_two_line_layout() {
        let mut output = Vec::new();
        let query = departure_query(vec![selected()], now(), false);
        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        let output = String::from_utf8(output).unwrap();
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("158  Centre"));
        assert!(lines[0].ends_with("leave in 4 min"));
        assert!(lines[1].starts_with("     Nearby stop · A"));
        assert!(lines[1].ends_with(&format!(
            "departs {}",
            local_clock(selected().departure.effective_at())
        )));
    }

    #[test]
    fn text_output_separates_full_width_records() {
        let query = departure_query(vec![selected(), selected()], now(), false);
        let mut output = Vec::new();

        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        let output = String::from_utf8(output).unwrap();
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[2], "─".repeat(lines[0].width()));
    }

    #[test]
    fn text_output_appends_cancellations_without_using_departure_slots() {
        let mut cancelled = selected();
        cancelled.departure.is_cancelled = true;
        let mut query = departure_query(vec![selected()], now(), false);
        query.cancelled.push(cancelled);
        let mut output = Vec::new();

        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        let output = String::from_utf8(output).unwrap();
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines[2], "─".repeat(lines[0].width()));
        assert_eq!(
            lines[3],
            format!(
                "cancelled: 158 → Centre from Nearby stop, {}",
                local_clock(selected().departure.effective_at())
            )
        );
    }

    #[test]
    fn text_output_lists_cancellations_after_the_empty_state() {
        let mut cancelled = selected();
        cancelled.departure.is_cancelled = true;
        let mut query = departure_query(Vec::new(), now(), false);
        query.cancelled.push(cancelled);
        let mut output = Vec::new();

        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!(
                "No reachable departures.\ncancelled: 158 → Centre from Nearby stop, {}\n",
                local_clock(selected().departure.effective_at())
            )
        );
    }

    #[test]
    fn display_width_uses_terminal_cells_for_unicode() {
        assert_eq!("Letňany".width(), 7);
        assert_eq!("Ａ".width(), 2);
        assert_eq!("e\u{301}".width(), 1);
    }

    #[test]
    fn styled_text_emphasizes_primary_and_dims_secondary_content() {
        let mut output = Vec::new();
        let query = departure_query(vec![selected()], now(), false);

        write_departures(&mut output, OutputFormat::Text, &query, true).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(&format!("{LINE_STYLE}158{RESET_STYLE}")));
        assert!(output.contains(&format!("{EMPHASIS_STYLE}Centre")));
        assert!(output.contains(&format!("{DIM_STYLE}     Nearby stop")));
    }

    #[test]
    fn text_output_handles_leave_now_and_no_results() {
        let mut departure = selected();
        departure.leave_in_seconds = 59;
        let mut output = Vec::new();
        let query = departure_query(vec![departure], now(), false);
        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();
        assert!(String::from_utf8(output).unwrap().contains("leave now"));

        let mut empty = Vec::new();
        let query = departure_query(Vec::new(), now(), false);
        write_departures(&mut empty, OutputFormat::Text, &query, false).unwrap();
        assert_eq!(
            String::from_utf8(empty).unwrap(),
            "No reachable departures.\n"
        );
    }

    #[test]
    fn json_output_has_a_machine_readable_envelope() {
        let mut output = Vec::new();
        let query = departure_query(vec![selected()], now() - TimeDelta::minutes(3), true);
        write_departures(&mut output, OutputFormat::Json, &query, false).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

        assert_eq!(json["schema_version"], JSON_SCHEMA_VERSION);
        assert_eq!(json["generated_at"], "2026-09-04T10:00:00Z");
        assert_eq!(json["data_updated_at"], "2026-09-04T09:57:00Z");
        assert_eq!(json["stale"], true);
        assert_eq!(json["departures"][0]["departure"]["line"], "158");
        assert_eq!(json["departures"][0]["leave_in_seconds"], 270);
        assert_eq!(json["cancelled"], serde_json::json!([]));
    }

    #[test]
    fn json_output_includes_cancelled_departures() {
        let mut cancelled = selected();
        cancelled.departure.is_cancelled = true;
        let mut query = departure_query(Vec::new(), now(), false);
        query.cancelled.push(cancelled);
        let mut output = Vec::new();

        write_departures(&mut output, OutputFormat::Json, &query, false).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

        assert_eq!(json["schema_version"], 2);
        assert_eq!(json["departures"], serde_json::json!([]));
        assert_eq!(json["cancelled"][0]["departure"]["is_cancelled"], true);
    }

    #[test]
    fn text_output_labels_stale_data_with_its_age() {
        let mut output = Vec::new();
        let query = departure_query(vec![selected()], now() - TimeDelta::seconds(190), true);
        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        assert!(String::from_utf8(output).unwrap().starts_with(&format!(
            "STALE · last updated {} · 3 min ago\n",
            local_clock(now() - TimeDelta::seconds(190))
        )));
    }

    #[test]
    fn stale_age_uses_human_scale_boundaries() {
        for (seconds, expected) in [
            (0, "just now"),
            (59, "just now"),
            (60, "1 min ago"),
            (3599, "59 min ago"),
            (3600, "1h 0m ago"),
            (7800, "2h 10m ago"),
            (86399, "23h 59m ago"),
            (86400, "1d ago"),
        ] {
            assert_eq!(stale_age_label(seconds), expected);
        }
    }

    #[test]
    fn delay_labels_only_material_late_and_early_running() {
        for (delay, expected) in [
            (None, None),
            (Some(0), None),
            (Some(59), None),
            (Some(60), Some("+1 late")),
            (Some(125), Some("+2 late")),
            (Some(-59), None),
            (Some(-60), Some("-1 early")),
            (Some(-125), Some("-2 early")),
        ] {
            assert_eq!(delay_label(delay).as_deref(), expected);
        }
    }

    #[test]
    fn text_output_includes_material_delay_on_the_secondary_line() {
        let mut departure = selected();
        departure.departure.delay_seconds = Some(125);
        let query = departure_query(vec![departure], now(), false);
        let mut output = Vec::new();

        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        let expected = format!(
            "departs {} · +2 late",
            local_clock(selected().departure.effective_at())
        );
        assert!(String::from_utf8(output).unwrap().contains(&expected));
    }
}
