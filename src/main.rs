use std::ffi::CString;
use std::fmt;
use std::io;
use std::mem;
use std::os::fd::RawFd;
use std::ptr;
use std::slice;

use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

const NETDEV_FAMILY_NAME: &str = "netdev";
const NETDEV_FAMILY_VERSION: u8 = 1;
const NETDEV_CMD_DEV_GET: u8 = 1;
const NETDEV_A_DEV_IFINDEX: u16 = 1;
const NETDEV_A_DEV_XDP_FEATURES: u16 = 3;
const NETDEV_A_DEV_XDP_ZC_MAX_SEGS: u16 = 4;
const NETDEV_A_DEV_XDP_RX_METADATA_FEATURES: u16 = 5;
const NETDEV_A_DEV_XSK_FEATURES: u16 = 6;

// These NETDEV_* bits are from linux/netdev.h and are distinct from the AF_XDP
// socket flags exported by libc, such as libc::XDP_ZEROCOPY.
const NETDEV_XDP_ACT_BASIC: u64 = 1;
const NETDEV_XDP_ACT_REDIRECT: u64 = 2;
const NETDEV_XDP_ACT_NDO_XMIT: u64 = 4;
const NETDEV_XDP_ACT_XSK_ZEROCOPY: u64 = 8;
const NETDEV_XDP_ACT_HW_OFFLOAD: u64 = 16;
const NETDEV_XDP_ACT_RX_SG: u64 = 32;
const NETDEV_XDP_ACT_NDO_XMIT_SG: u64 = 64;
const NETDEV_XDP_RX_METADATA_TIMESTAMP: u64 = 1;
const NETDEV_XDP_RX_METADATA_HASH: u64 = 2;
const NETDEV_XDP_RX_METADATA_VLAN_TAG: u64 = 4;
const NETDEV_XSK_FLAGS_TX_TIMESTAMP: u64 = 1;
const NETDEV_XSK_FLAGS_TX_CHECKSUM: u64 = 2;
const NLA_TYPE_MASK: u16 = 0x3fff;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum XdpFeature {
    #[value(name = "NETDEV_XDP_ACT_BASIC")]
    Basic,
    #[value(name = "NETDEV_XDP_ACT_REDIRECT")]
    Redirect,
    #[value(name = "NETDEV_XDP_ACT_NDO_XMIT")]
    NdoXmit,
    #[value(name = "NETDEV_XDP_ACT_XSK_ZEROCOPY")]
    XskZerocopy,
    #[value(name = "NETDEV_XDP_ACT_HW_OFFLOAD")]
    HwOffload,
    #[value(name = "NETDEV_XDP_ACT_RX_SG")]
    RxSg,
    #[value(name = "NETDEV_XDP_ACT_NDO_XMIT_SG")]
    NdoXmitSg,
    #[value(name = "NETDEV_A_DEV_XDP_ZC_MAX_SEGS")]
    ZcMaxSegs,
    #[value(name = "NETDEV_XDP_RX_METADATA_TIMESTAMP")]
    RxMetadataTimestamp,
    #[value(name = "NETDEV_XDP_RX_METADATA_HASH")]
    RxMetadataHash,
    #[value(name = "NETDEV_XDP_RX_METADATA_VLAN_TAG")]
    RxMetadataVlanTag,
    #[value(name = "NETDEV_XSK_FLAGS_TX_TIMESTAMP")]
    XskTxTimestamp,
    #[value(name = "NETDEV_XSK_FLAGS_TX_CHECKSUM")]
    XskTxChecksum,
}

impl XdpFeature {
    const ALL: [Self; 13] = [
        Self::Basic,
        Self::Redirect,
        Self::NdoXmit,
        Self::XskZerocopy,
        Self::HwOffload,
        Self::RxSg,
        Self::NdoXmitSg,
        Self::ZcMaxSegs,
        Self::RxMetadataTimestamp,
        Self::RxMetadataHash,
        Self::RxMetadataVlanTag,
        Self::XskTxTimestamp,
        Self::XskTxChecksum,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "NETDEV_XDP_ACT_BASIC",
            Self::Redirect => "NETDEV_XDP_ACT_REDIRECT",
            Self::NdoXmit => "NETDEV_XDP_ACT_NDO_XMIT",
            Self::XskZerocopy => "NETDEV_XDP_ACT_XSK_ZEROCOPY",
            Self::HwOffload => "NETDEV_XDP_ACT_HW_OFFLOAD",
            Self::RxSg => "NETDEV_XDP_ACT_RX_SG",
            Self::NdoXmitSg => "NETDEV_XDP_ACT_NDO_XMIT_SG",
            Self::ZcMaxSegs => "NETDEV_A_DEV_XDP_ZC_MAX_SEGS",
            Self::RxMetadataTimestamp => "NETDEV_XDP_RX_METADATA_TIMESTAMP",
            Self::RxMetadataHash => "NETDEV_XDP_RX_METADATA_HASH",
            Self::RxMetadataVlanTag => "NETDEV_XDP_RX_METADATA_VLAN_TAG",
            Self::XskTxTimestamp => "NETDEV_XSK_FLAGS_TX_TIMESTAMP",
            Self::XskTxChecksum => "NETDEV_XSK_FLAGS_TX_CHECKSUM",
        }
    }

    fn bit_source(self) -> FeatureBitSource {
        match self {
            Self::Basic => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_BASIC),
            Self::Redirect => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_REDIRECT),
            Self::NdoXmit => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_NDO_XMIT),
            Self::XskZerocopy => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_XSK_ZEROCOPY),
            Self::HwOffload => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_HW_OFFLOAD),
            Self::RxSg => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_RX_SG),
            Self::NdoXmitSg => FeatureBitSource::XdpActions(NETDEV_XDP_ACT_NDO_XMIT_SG),
            Self::ZcMaxSegs => FeatureBitSource::ZcMaxSegs,
            Self::RxMetadataTimestamp => {
                FeatureBitSource::RxMetadata(NETDEV_XDP_RX_METADATA_TIMESTAMP)
            }
            Self::RxMetadataHash => FeatureBitSource::RxMetadata(NETDEV_XDP_RX_METADATA_HASH),
            Self::RxMetadataVlanTag => {
                FeatureBitSource::RxMetadata(NETDEV_XDP_RX_METADATA_VLAN_TAG)
            }
            Self::XskTxTimestamp => FeatureBitSource::Xsk(NETDEV_XSK_FLAGS_TX_TIMESTAMP),
            Self::XskTxChecksum => FeatureBitSource::Xsk(NETDEV_XSK_FLAGS_TX_CHECKSUM),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FeatureBitSource {
    XdpActions(u64),
    ZcMaxSegs,
    RxMetadata(u64),
    Xsk(u64),
}

#[derive(Debug, Parser)]
#[command(
    name = "xdp-features",
    version,
    about = "Check whether an interface supports requested XDP features"
)]
struct Cli {
    #[arg(
        short,
        long,
        value_name = "IFNAME",
        global = true,
        help = "Network interface to inspect (required)"
    )]
    interface: Option<String>,

    #[arg(
        short,
        long,
        global = true,
        action = clap::ArgAction::Count,
        help = "Increase diagnostic verbosity"
    )]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Check whether all listed XDP features are supported")]
    Features(FeaturesArgs),
}

#[derive(Debug, Args)]
struct FeaturesArgs {
    #[arg(
        value_name = "FEATURE",
        required = true,
        num_args = 1..,
        help = "XDP feature required on the selected interface"
    )]
    feature: Vec<XdpFeature>,
}

fn main() {
    let cli = Cli::parse();
    let interface = cli.interface.as_deref().unwrap_or_else(|| {
        missing_interface_error().exit();
    });
    let features = selected_features(&cli);

    let supported = match run_features(interface, features, cli.verbose) {
        Ok(supported) => supported,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };

    if !supported {
        std::process::exit(1);
    }
}

fn selected_features(cli: &Cli) -> &[XdpFeature] {
    match &cli.command {
        Some(Commands::Features(args)) => &args.feature,
        None => &XdpFeature::ALL,
    }
}

fn missing_interface_error() -> clap::Error {
    let mut command = Cli::command();
    command.error(
        ErrorKind::MissingRequiredArgument,
        "the following required argument was not provided: --interface <IFNAME>",
    )
}

fn run_features(interface: &str, features: &[XdpFeature], verbose: u8) -> Result<bool, QueryError> {
    let query = query_xdp_features(interface)?;

    if verbose > 0 {
        println!("ifindex={}", query.ifindex);
        println!("xdp_features=0x{:016x}", query.xdp_features);
        println!(
            "xdp_zc_max_segs={}",
            query
                .xdp_zc_max_segs
                .map_or_else(|| "n/a".to_owned(), |value| value.to_string())
        );
        println!(
            "xdp_rx_metadata_features=0x{:016x}",
            query.xdp_rx_metadata_features.unwrap_or(0)
        );
        println!("xsk_features=0x{:016x}", query.xsk_features.unwrap_or(0));
    }

    let mut all_supported = true;
    for feature in features {
        let supported = print_feature(&query, *feature);
        all_supported = all_supported && supported;
    }

    Ok(all_supported)
}

fn print_feature(query: &XdpFeatureQuery, feature: XdpFeature) -> bool {
    if feature == XdpFeature::ZcMaxSegs {
        let supported = query.xdp_zc_max_segs.is_some();
        println!(
            "{}: {}",
            feature.as_str(),
            query
                .xdp_zc_max_segs
                .map_or_else(|| "n/a".to_owned(), |value| value.to_string())
        );
        return supported;
    }

    let supported = feature_supported(query, feature);
    println!(
        "{}: {}",
        feature.as_str(),
        if supported { "yes" } else { "no" }
    );
    supported
}

fn feature_supported(query: &XdpFeatureQuery, feature: XdpFeature) -> bool {
    match feature.bit_source() {
        FeatureBitSource::XdpActions(bit) => bit_supported(query.xdp_features, bit),
        FeatureBitSource::ZcMaxSegs => query.xdp_zc_max_segs.is_some(),
        FeatureBitSource::RxMetadata(bit) => {
            bit_supported(query.xdp_rx_metadata_features.unwrap_or(0), bit)
        }
        FeatureBitSource::Xsk(bit) => bit_supported(query.xsk_features.unwrap_or(0), bit),
    }
}

fn bit_supported(features: u64, bit: u64) -> bool {
    features & bit == bit
}

#[derive(Debug)]
struct XdpFeatureQuery {
    ifindex: u32,
    xdp_features: u64,
    xdp_zc_max_segs: Option<u32>,
    xdp_rx_metadata_features: Option<u64>,
    xsk_features: Option<u64>,
}

#[derive(Debug)]
enum QueryError {
    CString(std::ffi::NulError),
    InterfaceNotFound(String),
    Io(io::Error),
    Netlink(String),
    MissingFamily(String),
    MissingAttribute(&'static str),
    InvalidAttribute(&'static str),
}

impl fmt::Display for QueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CString(error) => write!(formatter, "{error}"),
            Self::InterfaceNotFound(interface) => {
                write!(formatter, "interface not found: {interface}")
            }
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Netlink(message) => write!(formatter, "{message}"),
            Self::MissingFamily(family) => {
                write!(formatter, "generic netlink family not found: {family}")
            }
            Self::MissingAttribute(attribute) => {
                write!(formatter, "netlink response did not include {attribute}")
            }
            Self::InvalidAttribute(attribute) => {
                write!(formatter, "netlink response included invalid {attribute}")
            }
        }
    }
}

impl std::error::Error for QueryError {}

impl From<io::Error> for QueryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<std::ffi::NulError> for QueryError {
    fn from(error: std::ffi::NulError) -> Self {
        Self::CString(error)
    }
}

fn query_xdp_features(interface: &str) -> Result<XdpFeatureQuery, QueryError> {
    let ifindex = interface_index(interface)?;
    let mut socket = NetlinkSocket::connect()?;
    let family_id = resolve_netdev_family_id(&mut socket)?;
    let messages = socket.request(
        family_id,
        NETDEV_CMD_DEV_GET,
        NETDEV_FAMILY_VERSION,
        &[NetlinkAttribute::u32(NETDEV_A_DEV_IFINDEX, ifindex)],
    )?;

    let mut xdp_features = None;
    let mut xdp_zc_max_segs = None;
    let mut xdp_rx_metadata_features = None;
    let mut xsk_features = None;

    for message in messages {
        let attrs = parse_genl_attrs(&message)?;
        for attr in attrs {
            match attr.kind {
                NETDEV_A_DEV_XDP_FEATURES => {
                    xdp_features = Some(read_u64_attr(attr.value, "NETDEV_A_DEV_XDP_FEATURES")?);
                }
                NETDEV_A_DEV_XDP_ZC_MAX_SEGS => {
                    xdp_zc_max_segs =
                        Some(read_u32_attr(attr.value, "NETDEV_A_DEV_XDP_ZC_MAX_SEGS")?);
                }
                NETDEV_A_DEV_XDP_RX_METADATA_FEATURES => {
                    xdp_rx_metadata_features = Some(read_u64_attr(
                        attr.value,
                        "NETDEV_A_DEV_XDP_RX_METADATA_FEATURES",
                    )?);
                }
                NETDEV_A_DEV_XSK_FEATURES => {
                    xsk_features = Some(read_u64_attr(attr.value, "NETDEV_A_DEV_XSK_FEATURES")?);
                }
                _ => {}
            }
        }
    }

    let xdp_features =
        xdp_features.ok_or(QueryError::MissingAttribute("NETDEV_A_DEV_XDP_FEATURES"))?;

    Ok(XdpFeatureQuery {
        ifindex,
        xdp_features,
        xdp_zc_max_segs,
        xdp_rx_metadata_features,
        xsk_features,
    })
}

fn interface_index(interface: &str) -> Result<u32, QueryError> {
    let c_interface = CString::new(interface)?;
    let ifindex = unsafe { libc::if_nametoindex(c_interface.as_ptr()) };

    if ifindex == 0 {
        return Err(QueryError::InterfaceNotFound(interface.to_owned()));
    }

    Ok(ifindex)
}

fn resolve_netdev_family_id(socket: &mut NetlinkSocket) -> Result<u16, QueryError> {
    let messages = socket.request(
        libc::GENL_ID_CTRL
            .try_into()
            .map_err(|_| QueryError::Netlink("invalid GENL_ID_CTRL constant".to_owned()))?,
        libc::CTRL_CMD_GETFAMILY
            .try_into()
            .map_err(|_| QueryError::Netlink("invalid CTRL_CMD_GETFAMILY constant".to_owned()))?,
        1,
        &[NetlinkAttribute::string(
            libc::CTRL_ATTR_FAMILY_NAME.try_into().map_err(|_| {
                QueryError::Netlink("invalid CTRL_ATTR_FAMILY_NAME constant".to_owned())
            })?,
            NETDEV_FAMILY_NAME,
        )?],
    )?;

    let family_id_attr: u16 = libc::CTRL_ATTR_FAMILY_ID
        .try_into()
        .map_err(|_| QueryError::Netlink("invalid CTRL_ATTR_FAMILY_ID constant".to_owned()))?;

    for message in messages {
        let attrs = parse_genl_attrs(&message)?;
        if let Some(attr) = attrs.iter().find(|attr| attr.kind == family_id_attr) {
            return read_u16_attr(attr.value, "CTRL_ATTR_FAMILY_ID");
        }
    }

    Err(QueryError::MissingFamily(NETDEV_FAMILY_NAME.to_owned()))
}

struct NetlinkSocket {
    fd: RawFd,
    sequence: u32,
}

impl NetlinkSocket {
    fn connect() -> Result<Self, QueryError> {
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                libc::NETLINK_GENERIC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }

        let socket = Self { fd, sequence: 0 };
        socket.bind()?;
        Ok(socket)
    }

    fn bind(&self) -> Result<(), QueryError> {
        let mut address = zeroed_sockaddr_nl();
        address.nl_family = libc::AF_NETLINK
            .try_into()
            .map_err(|_| QueryError::Netlink("invalid AF_NETLINK constant".to_owned()))?;
        address.nl_pid = 0;
        address.nl_groups = 0;

        let result = unsafe {
            libc::bind(
                self.fd,
                ptr::from_ref(&address).cast::<libc::sockaddr>(),
                socklen_of::<libc::sockaddr_nl>()?,
            )
        };

        if result < 0 {
            return Err(io::Error::last_os_error().into());
        }

        Ok(())
    }

    fn request(
        &mut self,
        message_type: u16,
        command: u8,
        version: u8,
        attrs: &[NetlinkAttribute],
    ) -> Result<Vec<Vec<u8>>, QueryError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| QueryError::Netlink("netlink sequence overflow".to_owned()))?;
        let request = build_genl_request(message_type, self.sequence, command, version, attrs)?;
        self.send(&request)?;
        self.receive(self.sequence)
    }

    fn send(&self, request: &[u8]) -> Result<(), QueryError> {
        let mut address = zeroed_sockaddr_nl();
        address.nl_family = libc::AF_NETLINK
            .try_into()
            .map_err(|_| QueryError::Netlink("invalid AF_NETLINK constant".to_owned()))?;

        let sent = unsafe {
            libc::sendto(
                self.fd,
                request.as_ptr().cast(),
                request.len(),
                0,
                ptr::from_ref(&address).cast::<libc::sockaddr>(),
                socklen_of::<libc::sockaddr_nl>()?,
            )
        };

        if sent < 0 {
            return Err(io::Error::last_os_error().into());
        }

        let sent: usize = sent
            .try_into()
            .map_err(|_| QueryError::Netlink("negative send length".to_owned()))?;
        if sent != request.len() {
            return Err(QueryError::Netlink("partial netlink send".to_owned()));
        }

        Ok(())
    }

    fn receive(&self, sequence: u32) -> Result<Vec<Vec<u8>>, QueryError> {
        let mut messages = Vec::new();

        loop {
            let mut buffer = vec![0_u8; 65_536];
            let received =
                unsafe { libc::recv(self.fd, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
            if received < 0 {
                return Err(io::Error::last_os_error().into());
            }

            let received: usize = received
                .try_into()
                .map_err(|_| QueryError::Netlink("negative receive length".to_owned()))?;
            if received == 0 {
                return Err(QueryError::Netlink("netlink socket closed".to_owned()));
            }
            buffer.truncate(received);

            let done = parse_netlink_messages(&buffer, sequence, &mut messages)?;
            if done {
                return Ok(messages);
            }

            if !messages.is_empty() {
                return Ok(messages);
            }
        }
    }
}

impl Drop for NetlinkSocket {
    fn drop(&mut self) {
        let _ = unsafe { libc::close(self.fd) };
    }
}

#[derive(Debug)]
struct NetlinkAttribute {
    kind: u16,
    value: Vec<u8>,
}

impl NetlinkAttribute {
    fn string(kind: u16, value: &str) -> Result<Self, QueryError> {
        let value = CString::new(value)?.into_bytes_with_nul();
        Ok(Self { kind, value })
    }

    fn u32(kind: u16, value: u32) -> Self {
        Self {
            kind,
            value: value.to_ne_bytes().to_vec(),
        }
    }
}

#[derive(Debug)]
struct ParsedAttribute<'a> {
    kind: u16,
    value: &'a [u8],
}

fn build_genl_request(
    message_type: u16,
    sequence: u32,
    command: u8,
    version: u8,
    attrs: &[NetlinkAttribute],
) -> Result<Vec<u8>, QueryError> {
    let header_len = mem::size_of::<libc::nlmsghdr>();
    let genl_len = align4(mem::size_of::<libc::genlmsghdr>())?;
    let mut buffer = vec![0_u8; checked_add(header_len, genl_len)?];

    let genl = libc::genlmsghdr {
        cmd: command,
        version,
        reserved: 0,
    };
    write_struct_at(&mut buffer, header_len, &genl)?;

    for attr in attrs {
        append_attr(&mut buffer, attr)?;
    }

    let nlmsg_len: u32 = buffer
        .len()
        .try_into()
        .map_err(|_| QueryError::Netlink("netlink request too large".to_owned()))?;
    let nlmsg_flags: u16 = libc::NLM_F_REQUEST
        .try_into()
        .map_err(|_| QueryError::Netlink("invalid NLM_F_REQUEST constant".to_owned()))?;
    let header = libc::nlmsghdr {
        nlmsg_len,
        nlmsg_type: message_type,
        nlmsg_flags,
        nlmsg_seq: sequence,
        nlmsg_pid: 0,
    };
    write_struct_at(&mut buffer, 0, &header)?;

    Ok(buffer)
}

fn append_attr(buffer: &mut Vec<u8>, attr: &NetlinkAttribute) -> Result<(), QueryError> {
    let header_len = mem::size_of::<libc::nlattr>();
    let attr_len = checked_add(header_len, attr.value.len())?;
    let nla_len: u16 = attr_len
        .try_into()
        .map_err(|_| QueryError::Netlink("netlink attribute too large".to_owned()))?;
    let header = libc::nlattr {
        nla_len,
        nla_type: attr.kind,
    };

    append_struct(buffer, &header);
    buffer.extend_from_slice(&attr.value);

    let aligned_len = align4(attr_len)?;
    let padded_len = checked_add(buffer.len(), aligned_len.saturating_sub(attr_len))?;
    buffer.resize(padded_len, 0);

    Ok(())
}

fn parse_netlink_messages(
    buffer: &[u8],
    sequence: u32,
    messages: &mut Vec<Vec<u8>>,
) -> Result<bool, QueryError> {
    let header_len = mem::size_of::<libc::nlmsghdr>();
    let mut offset = 0_usize;

    while checked_add(offset, header_len)? <= buffer.len() {
        let header: libc::nlmsghdr = read_struct_at(buffer, offset)?;
        let message_len: usize = header
            .nlmsg_len
            .try_into()
            .map_err(|_| QueryError::Netlink("invalid netlink message length".to_owned()))?;
        if message_len < header_len {
            return Err(QueryError::Netlink("short netlink message".to_owned()));
        }

        let message_end = checked_add(offset, message_len)?;
        if message_end > buffer.len() {
            return Err(QueryError::Netlink("truncated netlink message".to_owned()));
        }

        if header.nlmsg_seq != sequence {
            offset = checked_add(offset, align4(message_len)?)?;
            continue;
        }

        let payload_start = checked_add(offset, header_len)?;
        let payload = &buffer[payload_start..message_end];
        match i32::from(header.nlmsg_type) {
            kind if kind == libc::NLMSG_ERROR => handle_netlink_error(payload)?,
            kind if kind == libc::NLMSG_DONE => return Ok(true),
            _ => messages.push(payload.to_vec()),
        }

        offset = checked_add(offset, align4(message_len)?)?;
    }

    Ok(false)
}

fn handle_netlink_error(payload: &[u8]) -> Result<(), QueryError> {
    let errno = read_i32(payload, "NLMSG_ERROR")?;
    if errno == 0 {
        return Ok(());
    }

    let os_error = errno
        .checked_neg()
        .ok_or_else(|| QueryError::Netlink("invalid netlink errno".to_owned()))?;
    Err(io::Error::from_raw_os_error(os_error).into())
}

fn parse_genl_attrs(message: &[u8]) -> Result<Vec<ParsedAttribute<'_>>, QueryError> {
    let genl_len = align4(mem::size_of::<libc::genlmsghdr>())?;
    if message.len() < genl_len {
        return Err(QueryError::Netlink(
            "short generic netlink message".to_owned(),
        ));
    }

    parse_attrs(&message[genl_len..])
}

fn parse_attrs(mut payload: &[u8]) -> Result<Vec<ParsedAttribute<'_>>, QueryError> {
    let header_len = mem::size_of::<libc::nlattr>();
    let mut attrs = Vec::new();

    while payload.len() >= header_len {
        let header: libc::nlattr = read_struct_at(payload, 0)?;
        let attr_len = usize::from(header.nla_len);
        if attr_len < header_len {
            return Err(QueryError::Netlink("short netlink attribute".to_owned()));
        }
        if attr_len > payload.len() {
            return Err(QueryError::Netlink(
                "truncated netlink attribute".to_owned(),
            ));
        }

        attrs.push(ParsedAttribute {
            kind: header.nla_type & NLA_TYPE_MASK,
            value: &payload[header_len..attr_len],
        });

        let next = align4(attr_len)?;
        if next > payload.len() {
            return Err(QueryError::Netlink(
                "truncated netlink attribute padding".to_owned(),
            ));
        }
        payload = &payload[next..];
    }

    if payload.iter().any(|byte| *byte != 0) {
        return Err(QueryError::Netlink(
            "trailing netlink attribute data".to_owned(),
        ));
    }

    Ok(attrs)
}

fn read_u16_attr(value: &[u8], name: &'static str) -> Result<u16, QueryError> {
    let bytes = value
        .get(..mem::size_of::<u16>())
        .ok_or(QueryError::InvalidAttribute(name))?;
    let bytes: [u8; 2] = bytes
        .try_into()
        .map_err(|_| QueryError::InvalidAttribute(name))?;
    Ok(u16::from_ne_bytes(bytes))
}

fn read_u32_attr(value: &[u8], name: &'static str) -> Result<u32, QueryError> {
    let bytes = value
        .get(..mem::size_of::<u32>())
        .ok_or(QueryError::InvalidAttribute(name))?;
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| QueryError::InvalidAttribute(name))?;
    Ok(u32::from_ne_bytes(bytes))
}

fn read_u64_attr(value: &[u8], name: &'static str) -> Result<u64, QueryError> {
    let bytes = value
        .get(..mem::size_of::<u64>())
        .ok_or(QueryError::InvalidAttribute(name))?;
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| QueryError::InvalidAttribute(name))?;
    Ok(u64::from_ne_bytes(bytes))
}

fn read_i32(value: &[u8], name: &'static str) -> Result<i32, QueryError> {
    let bytes = value
        .get(..mem::size_of::<i32>())
        .ok_or(QueryError::InvalidAttribute(name))?;
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| QueryError::InvalidAttribute(name))?;
    Ok(i32::from_ne_bytes(bytes))
}

fn read_struct_at<T: Copy>(buffer: &[u8], offset: usize) -> Result<T, QueryError> {
    let size = mem::size_of::<T>();
    let end = checked_add(offset, size)?;
    if end > buffer.len() {
        return Err(QueryError::Netlink("short structured data".to_owned()));
    }

    // Netlink structures are naturally aligned in kernel messages, but slices do
    // not carry that guarantee. Use unaligned reads for parser robustness.
    Ok(unsafe { ptr::read_unaligned(buffer.as_ptr().add(offset).cast::<T>()) })
}

fn write_struct_at<T>(buffer: &mut [u8], offset: usize, value: &T) -> Result<(), QueryError> {
    let size = mem::size_of::<T>();
    let end = checked_add(offset, size)?;
    if end > buffer.len() {
        return Err(QueryError::Netlink("short output buffer".to_owned()));
    }

    let bytes = unsafe { slice::from_raw_parts(ptr::from_ref(value).cast::<u8>(), size) };
    buffer[offset..end].copy_from_slice(bytes);
    Ok(())
}

fn append_struct<T>(buffer: &mut Vec<u8>, value: &T) {
    let size = mem::size_of::<T>();
    let bytes = unsafe { slice::from_raw_parts(ptr::from_ref(value).cast::<u8>(), size) };
    buffer.extend_from_slice(bytes);
}

fn checked_add(left: usize, right: usize) -> Result<usize, QueryError> {
    left.checked_add(right)
        .ok_or_else(|| QueryError::Netlink("integer overflow".to_owned()))
}

fn align4(value: usize) -> Result<usize, QueryError> {
    value
        .checked_add(3)
        .map(|value| value & !3)
        .ok_or_else(|| QueryError::Netlink("integer overflow".to_owned()))
}

fn socklen_of<T>() -> Result<libc::socklen_t, QueryError> {
    mem::size_of::<T>()
        .try_into()
        .map_err(|_| QueryError::Netlink("socklen_t overflow".to_owned()))
}

fn zeroed_sockaddr_nl() -> libc::sockaddr_nl {
    unsafe { mem::zeroed() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_args_parse_for_missing_interface_error() {
        let cli = Cli::try_parse_from(["xdp-features"]).unwrap();

        assert_eq!(cli.interface, None);
        assert!(cli.command.is_none());
        assert_eq!(
            missing_interface_error().kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn missing_subcommand_selects_all_features() {
        let cli = Cli::try_parse_from(["xdp-features", "--interface", "eth0"]).unwrap();

        assert_eq!(cli.interface.as_deref(), Some("eth0"));
        assert!(cli.command.is_none());
        assert_eq!(selected_features(&cli), XdpFeature::ALL);
    }

    #[test]
    fn features_subcommand_requires_feature_argument() {
        let error =
            Cli::try_parse_from(["xdp-features", "--interface", "eth0", "features"]).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn interface_can_be_passed_after_subcommand() {
        let cli = Cli::try_parse_from([
            "xdp-features",
            "features",
            "NETDEV_XDP_ACT_BASIC",
            "NETDEV_A_DEV_XDP_ZC_MAX_SEGS",
            "NETDEV_XDP_RX_METADATA_HASH",
            "NETDEV_XSK_FLAGS_TX_CHECKSUM",
            "--interface",
            "eth0",
        ])
        .unwrap();

        assert_eq!(cli.interface.as_deref(), Some("eth0"));
        let Some(Commands::Features(args)) = cli.command else {
            panic!("features subcommand was not parsed");
        };
        assert_eq!(
            args.feature,
            vec![
                XdpFeature::Basic,
                XdpFeature::ZcMaxSegs,
                XdpFeature::RxMetadataHash,
                XdpFeature::XskTxChecksum
            ]
        );
    }

    #[test]
    fn missing_interface_uses_argument_error() {
        let error = missing_interface_error();

        assert_eq!(error.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn feature_bits_match_netdev_uapi() {
        assert_eq!(
            XdpFeature::Basic.bit_source(),
            FeatureBitSource::XdpActions(1)
        );
        assert_eq!(
            XdpFeature::Redirect.bit_source(),
            FeatureBitSource::XdpActions(2)
        );
        assert_eq!(
            XdpFeature::NdoXmit.bit_source(),
            FeatureBitSource::XdpActions(4)
        );
        assert_eq!(
            XdpFeature::XskZerocopy.bit_source(),
            FeatureBitSource::XdpActions(8)
        );
        assert_eq!(
            XdpFeature::HwOffload.bit_source(),
            FeatureBitSource::XdpActions(16)
        );
        assert_eq!(
            XdpFeature::RxSg.bit_source(),
            FeatureBitSource::XdpActions(32)
        );
        assert_eq!(
            XdpFeature::NdoXmitSg.bit_source(),
            FeatureBitSource::XdpActions(64)
        );
        assert_eq!(
            XdpFeature::ZcMaxSegs.bit_source(),
            FeatureBitSource::ZcMaxSegs
        );
        assert_eq!(
            XdpFeature::RxMetadataTimestamp.bit_source(),
            FeatureBitSource::RxMetadata(1)
        );
        assert_eq!(
            XdpFeature::RxMetadataHash.bit_source(),
            FeatureBitSource::RxMetadata(2)
        );
        assert_eq!(
            XdpFeature::RxMetadataVlanTag.bit_source(),
            FeatureBitSource::RxMetadata(4)
        );
        assert_eq!(
            XdpFeature::XskTxTimestamp.bit_source(),
            FeatureBitSource::Xsk(1)
        );
        assert_eq!(
            XdpFeature::XskTxChecksum.bit_source(),
            FeatureBitSource::Xsk(2)
        );
    }

    #[test]
    fn feature_support_checks_requested_bit() {
        let query = XdpFeatureQuery {
            ifindex: 7,
            xdp_features: NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_RX_SG,
            xdp_zc_max_segs: Some(8),
            xdp_rx_metadata_features: Some(
                NETDEV_XDP_RX_METADATA_TIMESTAMP | NETDEV_XDP_RX_METADATA_VLAN_TAG,
            ),
            xsk_features: Some(NETDEV_XSK_FLAGS_TX_CHECKSUM),
        };

        assert!(feature_supported(&query, XdpFeature::Basic));
        assert!(feature_supported(&query, XdpFeature::RxSg));
        assert!(feature_supported(&query, XdpFeature::ZcMaxSegs));
        assert!(feature_supported(&query, XdpFeature::RxMetadataTimestamp));
        assert!(feature_supported(&query, XdpFeature::RxMetadataVlanTag));
        assert!(feature_supported(&query, XdpFeature::XskTxChecksum));
        assert!(!feature_supported(&query, XdpFeature::Redirect));
        assert!(!feature_supported(&query, XdpFeature::RxMetadataHash));
        assert!(!feature_supported(&query, XdpFeature::XskTxTimestamp));
    }

    #[test]
    fn parses_aligned_netlink_attributes() {
        let mut payload = Vec::new();
        append_attr(&mut payload, &NetlinkAttribute::u32(7, 0x1122_3344)).unwrap();
        append_attr(
            &mut payload,
            &NetlinkAttribute {
                kind: 8,
                value: [0xaa, 0xbb, 0xcc].to_vec(),
            },
        )
        .unwrap();

        let attrs = parse_attrs(&payload).unwrap();

        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].kind, 7);
        assert_eq!(attrs[0].value, 0x1122_3344_u32.to_ne_bytes());
        assert_eq!(attrs[1].kind, 8);
        assert_eq!(attrs[1].value, [0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn rejects_short_netlink_attribute() {
        let payload = [1, 0, 7, 0];

        assert!(parse_attrs(&payload).is_err());
    }
}
