use std::io::Write;

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use pidjezdy_core::selection::SelectedDeparture;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Error)]
pub(crate) enum OutputError {
    #[error("could not serialize JSON output: {0}")]
    Json(#[from] serde_json::Error),
    #[error("could not write command output: {0}")]
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
    generated_at: DateTime<Utc>,
    data_updated_at: DateTime<Utc>,
    stale: bool,
    departures: &[SelectedDeparture],
    styled_text: bool,
) -> Result<(), OutputError> {
    match format {
        OutputFormat::Text => {
            write_text(
                output,
                generated_at,
                data_updated_at,
                stale,
                departures,
                styled_text,
            )?;
        }
        OutputFormat::Json => {
            let document = serde_json::to_string(&JsonOutput {
                generated_at,
                data_updated_at,
                stale,
                departures,
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
    let line_width = rows
        .iter()
        .map(|row| display_width(row.line))
        .max()
        .unwrap_or(0);
    let detail_width = rows
        .iter()
        .flat_map(|row| [display_width(row.headsign), display_width(&row.boarding)])
        .max()
        .unwrap_or(0);
    let time_width = rows
        .iter()
        .flat_map(|row| [display_width(&row.leave), display_width(&row.departs)])
        .max()
        .unwrap_or(0);
    let separator = "─".repeat(line_width + detail_width + time_width + 4);

    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            write_styled_line(output, styled, "\x1b[2m", &separator)?;
        }

        let line = pad_right(row.line, line_width);
        let headsign = pad_right(row.headsign, detail_width);
        let leave = pad_left(&row.leave, time_width);
        if styled {
            writeln!(
                output,
                "\x1b[1;36m{line}\x1b[0m  \x1b[1m{headsign}\x1b[0m  \x1b[1m{leave}\x1b[0m"
            )?;
        } else {
            writeln!(output, "{line}  {headsign}  {leave}")?;
        }

        let boarding = pad_right(&row.boarding, detail_width);
        let departs = pad_left(&row.departs, time_width);
        let secondary = format!("{}  {boarding}  {departs}", " ".repeat(line_width));
        write_styled_line(output, styled, "\x1b[2m", &secondary)?;
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

fn display_width(value: &str) -> usize {
    value.chars().count()
}

fn pad_right(value: &str, width: usize) -> String {
    format!(
        "{value}{}",
        " ".repeat(width.saturating_sub(display_width(value)))
    )
}

fn pad_left(value: &str, width: usize) -> String {
    format!(
        "{}{value}",
        " ".repeat(width.saturating_sub(display_width(value)))
    )
}

fn write_styled_line(
    output: &mut impl Write,
    styled: bool,
    style: &str,
    value: &str,
) -> Result<(), std::io::Error> {
    if styled {
        writeln!(output, "{style}{value}\x1b[0m")
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

    #[test]
    fn text_output_uses_aligned_two_line_layout() {
        let mut output = Vec::new();
        write_departures(
            &mut output,
            OutputFormat::Text,
            now(),
            now(),
            false,
            &[selected()],
            false,
        )
        .unwrap();

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
        let departures = [selected(), selected()];
        let mut output = Vec::new();

        write_departures(
            &mut output,
            OutputFormat::Text,
            now(),
            now(),
            false,
            &departures,
            false,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[2], "─".repeat(lines[0].chars().count()));
    }

    #[test]
    fn styled_text_emphasizes_primary_and_dims_secondary_content() {
        let mut output = Vec::new();

        write_departures(
            &mut output,
            OutputFormat::Text,
            now(),
            now(),
            false,
            &[selected()],
            true,
        )
        .unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\x1b[1;36m158\x1b[0m"));
        assert!(output.contains("\x1b[1mCentre"));
        assert!(output.contains("\x1b[2m     Nearby stop"));
    }

    #[test]
    fn text_output_handles_leave_now_and_no_results() {
        let mut departure = selected();
        departure.leave_in_seconds = 59;
        let mut output = Vec::new();
        write_departures(
            &mut output,
            OutputFormat::Text,
            now(),
            now(),
            false,
            &[departure],
            false,
        )
        .unwrap();
        assert!(String::from_utf8(output).unwrap().contains("leave now"));

        let mut empty = Vec::new();
        write_departures(
            &mut empty,
            OutputFormat::Text,
            now(),
            now(),
            false,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(empty).unwrap(),
            "No reachable departures.\n"
        );
    }

    #[test]
    fn json_output_has_a_machine_readable_envelope() {
        let mut output = Vec::new();
        write_departures(
            &mut output,
            OutputFormat::Json,
            now(),
            now() - TimeDelta::minutes(3),
            true,
            &[selected()],
            false,
        )
        .unwrap();
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
        write_departures(
            &mut output,
            OutputFormat::Text,
            now(),
            now() - TimeDelta::seconds(190),
            true,
            &[selected()],
            false,
        )
        .unwrap();

        assert!(
            String::from_utf8(output)
                .unwrap()
                .starts_with("STALE · data updated 3 min ago\n")
        );
    }
}
