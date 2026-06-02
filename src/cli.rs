use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "xdp-features",
    version,
    about = "Inspect and check XDP readiness for Linux network interfaces"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print a human-readable XDP readiness report.
    Info(InfoArgs),

    /// Check XDP readiness for scripts.
    Check(CheckArgs),
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Network interface to inspect. If omitted, inspect every interface.
    #[arg(short, long)]
    pub interface: Option<String>,

    /// Print raw netlink values and every known XDP feature flag.
    #[arg(long)]
    pub verbose: bool,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Network interface to check.
    #[arg(short, long)]
    pub interface: String,

    /// Also require AF_XDP zero-copy support.
    #[arg(long)]
    pub zero_copy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser, error::ErrorKind};

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn info_accepts_no_interface() {
        let cli = Cli::try_parse_from(["xdp-features", "info"]).unwrap();

        let Commands::Info(args) = cli.command else {
            panic!("expected info command");
        };
        assert_eq!(args.interface, None);
        assert!(!args.verbose);
    }

    #[test]
    fn info_accepts_interface_and_verbose() {
        let cli = Cli::try_parse_from(["xdp-features", "info", "--interface", "eth0", "--verbose"])
            .unwrap();

        let Commands::Info(args) = cli.command else {
            panic!("expected info command");
        };
        assert_eq!(args.interface.as_deref(), Some("eth0"));
        assert!(args.verbose);
    }

    #[test]
    fn check_requires_interface() {
        let error = Cli::try_parse_from(["xdp-features", "check"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn check_accepts_interface_and_zero_copy() {
        let cli =
            Cli::try_parse_from(["xdp-features", "check", "-i", "eth0", "--zero-copy"]).unwrap();

        let Commands::Check(args) = cli.command else {
            panic!("expected check command");
        };
        assert_eq!(args.interface, "eth0");
        assert!(args.zero_copy);
    }

    #[test]
    fn old_features_subcommand_is_rejected() {
        let error = Cli::try_parse_from(["xdp-features", "features"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidSubcommand);
    }
}
