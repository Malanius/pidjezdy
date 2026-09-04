use std::process::ExitCode;

fn main() -> ExitCode {
    match pidjezdy::run_from_env() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pidjezdy: {error}");
            ExitCode::FAILURE
        }
    }
}
