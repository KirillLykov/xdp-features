mod cli;
mod info;

use {cli::Cli, cli::Commands, std::process::ExitCode};

fn main() -> ExitCode {
    match run(Cli::parse_args()) {
        Ok(exit_code) => exit_code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, info::QueryError> {
    match cli.command {
        Commands::Info(args) => {
            let ready = match args.interface {
                Some(interface) => info::run_info(&interface, args.verbose)?,
                None => info::run_info_all(args.verbose)?,
            };
            Ok(exit_code_for_ready(ready))
        }
        Commands::Check(args) => Ok(if info::run_check(&args.interface, args.zero_copy) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }),
    }
}

fn exit_code_for_ready(ready: bool) -> ExitCode {
    if ready {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
