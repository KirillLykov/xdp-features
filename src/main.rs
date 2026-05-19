use clap::{Arg, ArgAction, Command};

const XDP_FEATURES: [&str; 7] = [
    "NETDEV_XDP_ACT_BASIC",
    "NETDEV_XDP_ACT_REDIRECT",
    "NETDEV_XDP_ACT_NDO_XMIT",
    "NETDEV_XDP_ACT_XSK_ZEROCOPY",
    "NETDEV_XDP_ACT_HW_OFFLOAD",
    "NETDEV_XDP_ACT_RX_SG",
    "NETDEV_XDP_ACT_NDO_XMIT_SG",
];

fn main() {
    let matches = cli().get_matches();
    let interface = matches
        .get_one::<String>("interface")
        .expect("interface is required by clap");
    let verbose = matches.get_count("verbose");

    match matches.subcommand() {
        Some(("features", subcommand)) => {
            let features = subcommand
                .get_many::<String>("feature")
                .expect("at least one feature is required by clap")
                .map(String::as_str)
                .collect::<Vec<_>>();

            run_features(interface, &features, verbose);
        }
        _ => unreachable!("subcommand is required by clap"),
    }
}

fn cli() -> Command {
    Command::new("xdp-features")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Check whether an interface supports requested XDP features")
        .arg_required_else_help(true)
        .subcommand_required(true)
        .arg(
            Arg::new("interface")
                .short('i')
                .long("interface")
                .value_name("IFNAME")
                .global(true)
                .required(true)
                .help("Network interface to inspect"),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .global(true)
                .action(ArgAction::Count)
                .help("Increase diagnostic verbosity"),
        )
        .subcommand(
            Command::new("features")
                .about("Check whether all listed XDP features are supported")
                .arg(
                    Arg::new("feature")
                        .value_name("FEATURE")
                        .required(true)
                        .num_args(1..)
                        .value_parser(XDP_FEATURES)
                        .help("XDP feature required on the selected interface"),
                ),
        )
}

fn run_features(interface: &str, features: &[&str], verbose: u8) {
    println!(
        "feature query is not implemented yet: interface={interface}, features={features:?}, verbose={verbose}"
    );
}
