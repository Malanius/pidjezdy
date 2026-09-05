use std::error::Error;
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    match pidjezdy::run_from_env() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = write_error_report(&mut std::io::stderr().lock(), &error);
            ExitCode::FAILURE
        }
    }
}

fn write_error_report(
    output: &mut impl Write,
    error: &(dyn Error + 'static),
) -> Result<(), std::io::Error> {
    let mut previous = error.to_string();
    writeln!(output, "pidjezdy: {previous}")?;

    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if !previous.ends_with(&message) {
            writeln!(output, "  caused by: {message}")?;
        }
        previous = message;
        source = cause.source();
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
}
