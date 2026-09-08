use std::error::Error;
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    match pidjezdy::run_from_env() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if !error.json_reported() {
                let _ = write_error_report(&mut std::io::stderr().lock(), &error);
            }
            ExitCode::FAILURE
        }
    }
}

fn write_error_report(
    output: &mut impl Write,
    error: &(dyn Error + 'static),
) -> Result<(), std::io::Error> {
    writeln!(output, "pidjezdy: {error}")?;
    for cause in pidjezdy::distinct_causes(error) {
        write_cause(output, &cause)?;
    }
    Ok(())
}

fn write_cause(output: &mut impl Write, message: &str) -> Result<(), std::io::Error> {
    let mut lines = message.lines();
    if let Some(first) = lines.next() {
        writeln!(output, "  caused by: {first}")?;
        for line in lines {
            writeln!(output, "             {line}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pidjezdy::{AppError, OutputError};

    use super::*;

    #[test]
    fn reports_each_distinct_error_layer() {
        let error = AppError::from(OutputError::Write(std::io::Error::other("closed output")));
        let mut output = Vec::new();

        write_error_report(&mut output, &error).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "pidjezdy: could not write command output\n  caused by: closed output\n"
        );
    }

    #[test]
    fn skips_a_source_already_included_by_its_wrapper() {
        let source = pidjezdy_core::config::Config::from_toml("not = [valid").unwrap_err();
        let error = AppError::InvalidConfig {
            path: PathBuf::from("config.toml"),
            source,
        };
        let mut output = Vec::new();

        write_error_report(&mut output, &error).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.starts_with("pidjezdy: invalid configuration config.toml\n"));
        assert!(output.contains("  caused by: invalid TOML:"));
        assert_eq!(output.matches("TOML parse error").count(), 1);
    }

    #[test]
    fn indents_every_line_of_a_multiline_cause() {
        let source = pidjezdy_core::config::Config::from_toml(
            r"
                [display]
                max_departures = 0
            ",
        )
        .unwrap_err();
        let error = AppError::InvalidConfig {
            path: PathBuf::from("config.toml"),
            source,
        };
        let mut output = Vec::new();

        write_error_report(&mut output, &error).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("  caused by: configuration is invalid:\n"));
        assert!(
            output
                .lines()
                .skip(2)
                .all(|line| line.starts_with("             - "))
        );
    }
}
