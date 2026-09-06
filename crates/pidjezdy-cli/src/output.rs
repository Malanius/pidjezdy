use std::io::Write;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use pidjezdy_core::selection::SelectedDeparture;
use serde::Serialize;
use thiserror::Error;
use unicode_width::UnicodeWidthStr;

use crate::departures::DepartureQuery;

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
    generated_at: DateTime<Utc>,
    data_updated_at: DateTime<Utc>,
    stale: bool,
    departures: &'a [SelectedDeparture],
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
                styled_text,
            )?;
        }
        OutputFormat::Json => {
            let document = serde_json::to_string(&JsonOutput {
                generated_at: query.generated_at,
                data_updated_at: query.data_updated_at,
                stale: query.stale,
                departures: &query.departures,
            })?;
            writeln!(output, "{document}")?;
        }
    }
    Ok(())
}

fn write_text(
    output: &mut impl Write,
    generated_at: DateTime<Utc>,
    data_updated_at: DateTime<Utc>,
    stale: bool,
    departures: &[SelectedDeparture],
    styled: bool,
) -> Result<(), std::io::Error> {
    if stale {
        let age_minutes = (generated_at - data_updated_at).num_minutes().max(0);
        writeln!(output, "STALE · data updated {age_minutes} min ago")?;
    }
    if departures.is_empty() {
        return writeln!(output, "No reachable departures.");
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
    let separator = "─".repeat(line_width + detail_width + time_width + 4);

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
    Ok(())
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
            .map_or_else(String::new, |code| format!(" · platform {code}"));
        let leave = if selected.leave_in_minutes() == 0 {
            "leave now".to_owned()
        } else {
            format!("leave in {} min", selected.leave_in_minutes())
        };

        Self {
            line: &departure.line,
            headsign: &departure.headsign,
            boarding: format!("{}{platform}", selected.boarding_point_name),
            leave,
            departs: format!("departs in {} min", selected.departs_in_minutes()),
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
            cache_warning: None,
        }
    }

    #[test]
    fn text_output_uses_aligned_two_line_layout() {
        let mut output = Vec::new();
        let query = departure_query(vec![selected()], now(), false);
        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "158  Centre                       leave in 4 min\n",
                "     Nearby stop · platform A  departs in 10 min\n"
            )
        );
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

        assert_eq!(json["generated_at"], "2026-09-04T10:00:00Z");
        assert_eq!(json["data_updated_at"], "2026-09-04T09:57:00Z");
        assert_eq!(json["stale"], true);
        assert_eq!(json["departures"][0]["departure"]["line"], "158");
        assert_eq!(json["departures"][0]["leave_in_seconds"], 270);
    }

    #[test]
    fn text_output_labels_stale_data_with_its_age() {
        let mut output = Vec::new();
        let query = departure_query(vec![selected()], now() - TimeDelta::seconds(190), true);
        write_departures(&mut output, OutputFormat::Text, &query, false).unwrap();

        assert!(
            String::from_utf8(output)
                .unwrap()
                .starts_with("STALE · data updated 3 min ago\n")
        );
    }
}
