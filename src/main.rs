use std::process::ExitCode;

fn main() -> ExitCode {
    match work_shmirk::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(1)
        }
    }
}
