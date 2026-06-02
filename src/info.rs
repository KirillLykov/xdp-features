use {
    agave_xdp::device::NetworkDevice,
    std::{
        ffi::CString, fmt, fs, io, mem, os::fd::RawFd, path::Path, process::Command, ptr, slice,
    },
};

const CHECK: &str = "✅";
const CROSS: &str = "❌";
const WARNING: &str = "⚠️";

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum XdpFeature {
    Basic,
    Redirect,
    NdoXmit,
    XskZerocopy,
    HwOffload,
    RxSg,
    NdoXmitSg,
    ZcMaxSegs,
    RxMetadataTimestamp,
    RxMetadataHash,
    RxMetadataVlanTag,
    XskTxTimestamp,
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

pub fn run_info(interface: &str, verbose: bool) -> Result<bool, QueryError> {
    let report = query_info_report(interface)?;
    print!("{}", render_report(&report, verbose));
    Ok(report.ready())
}

pub fn run_info_all(verbose: bool) -> Result<bool, QueryError> {
    let reports = list_interfaces()?
        .into_iter()
        .map(|interface| query_interface_info(&interface))
        .collect::<Vec<_>>();
    print!("{}", render_interface_reports(&reports, verbose));
    Ok(all_interface_reports_ready(&reports))
}

pub fn run_check(interface: &str, require_zero_copy: bool) -> bool {
    match query_check_report(interface, require_zero_copy) {
        Ok(report) => {
            let failures = report.failures();
            for failure in &failures {
                eprintln!("{failure}");
            }
            failures.is_empty()
        }
        Err(error) => {
            eprintln!("error: {error}");
            false
        }
    }
}

struct InfoReport {
    interface: String,
    nic_info: NicInfo,
    query: XdpFeatureQuery,
    ring_report: RingReport,
}

impl InfoReport {
    fn ready(&self) -> bool {
        xdp_supported(&self.query) && zero_copy_supported(&self.query) && self.ring_report.ready()
    }
}

struct CheckReport {
    interface: String,
    query: XdpFeatureQuery,
    ring_report: RingReport,
    require_zero_copy: bool,
}

impl CheckReport {
    fn failures(&self) -> Vec<String> {
        let mut failures = Vec::new();

        if !xdp_supported(&self.query) {
            failures.push("XDP not supported".to_string());
        }

        if self.require_zero_copy && !zero_copy_supported(&self.query) {
            failures.push("zero-copy not supported".to_string());
        }

        match &self.ring_report {
            RingReport::Sizes { tx, .. } => {
                if !tx.valid {
                    failures.push(render_invalid_tx_ring_failure(&self.interface, tx));
                }
            }
            RingReport::Unavailable(error) => {
                failures.push(format!("unable to query ring sizes: {error}"));
            }
        }

        failures
    }
}

struct InfoErrorReport {
    interface: String,
    nic_info: NicInfo,
    error: String,
}

enum InterfaceInfoReport {
    Report {
        kind: InterfaceKind,
        report: InfoReport,
    },
    Error {
        kind: InterfaceKind,
        report: InfoErrorReport,
    },
}

impl InterfaceInfoReport {
    fn kind(&self) -> InterfaceKind {
        match self {
            Self::Report { kind, .. } | Self::Error { kind, .. } => *kind,
        }
    }

    fn ready(&self) -> bool {
        match self {
            Self::Report { report, .. } => report.ready(),
            Self::Error { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterfaceKind {
    Physical,
    VirtualLogical,
}

impl InterfaceKind {
    fn heading(self) -> &'static str {
        match self {
            Self::Physical => "Physical interfaces:",
            Self::VirtualLogical => "Virtual/logical interfaces:",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct NicInfo {
    model: Option<String>,
    driver: Option<String>,
    pci_slot: Option<String>,
    pci_vendor: Option<String>,
    pci_device: Option<String>,
    pci_subsystem_vendor: Option<String>,
    pci_subsystem_device: Option<String>,
}

impl NicInfo {
    fn display_model(&self) -> Option<String> {
        self.model.clone().or_else(|| {
            let vendor = normalize_pci_id(self.pci_vendor.as_deref()?);
            let device = normalize_pci_id(self.pci_device.as_deref()?);
            Some(format!("PCI device [{vendor}:{device}]"))
        })
    }
}

enum RingReport {
    Sizes { tx: RingCheck, rx: RingCheck },
    Unavailable(io::Error),
}

impl RingReport {
    fn ready(&self) -> bool {
        match self {
            Self::Sizes { tx, .. } => tx.valid,
            Self::Unavailable(_) => true,
        }
    }
}

struct RingCheck {
    name: RingName,
    size: usize,
    valid: bool,
    recommended_size: Option<usize>,
}

#[derive(Clone, Copy)]
enum RingName {
    Tx,
    Rx,
}

impl RingName {
    fn upper(self) -> &'static str {
        match self {
            Self::Tx => "TX",
            Self::Rx => "RX",
        }
    }

    fn ethtool_arg(self) -> &'static str {
        match self {
            Self::Tx => "tx",
            Self::Rx => "rx",
        }
    }
}

fn query_ring_sizes(interface: &str) -> RingReport {
    match NetworkDevice::ring_sizes(interface) {
        Ok(sizes) => RingReport::Sizes {
            tx: check_ring_size(RingName::Tx, sizes.tx),
            rx: check_ring_size(RingName::Rx, sizes.rx),
        },
        Err(err) => RingReport::Unavailable(err),
    }
}

fn query_check_report(interface: &str, require_zero_copy: bool) -> Result<CheckReport, QueryError> {
    Ok(CheckReport {
        interface: interface.to_owned(),
        query: query_xdp_features(interface)?,
        ring_report: query_ring_sizes(interface),
        require_zero_copy,
    })
}

fn query_info_report(interface: &str) -> Result<InfoReport, QueryError> {
    Ok(InfoReport {
        interface: interface.to_owned(),
        nic_info: query_nic_info(interface),
        query: query_xdp_features(interface)?,
        ring_report: query_ring_sizes(interface),
    })
}

fn query_interface_info(interface: &str) -> InterfaceInfoReport {
    let kind = classify_interface(interface);
    match query_info_report(interface) {
        Ok(report) => InterfaceInfoReport::Report { kind, report },
        Err(error) => InterfaceInfoReport::Error {
            kind,
            report: InfoErrorReport {
                interface: interface.to_owned(),
                nic_info: query_nic_info(interface),
                error: error.to_string(),
            },
        },
    }
}

fn list_interfaces() -> Result<Vec<String>, QueryError> {
    let mut interfaces = Vec::new();
    for entry in fs::read_dir("/sys/class/net")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.is_empty() {
            interfaces.push(name);
        }
    }
    interfaces.sort();
    Ok(interfaces)
}

fn classify_interface(interface: &str) -> InterfaceKind {
    classify_interface_device_path(&Path::new("/sys/class/net").join(interface).join("device"))
}

fn classify_interface_device_path(device_path: &Path) -> InterfaceKind {
    fs::canonicalize(device_path).map_or(InterfaceKind::VirtualLogical, |path| {
        classify_canonical_device_path(&path)
    })
}

fn classify_canonical_device_path(path: &Path) -> InterfaceKind {
    if path.starts_with("/sys/devices/virtual/net") {
        return InterfaceKind::VirtualLogical;
    }

    if path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(looks_like_pci_slot)
    {
        InterfaceKind::Physical
    } else {
        InterfaceKind::VirtualLogical
    }
}

fn query_nic_info(interface: &str) -> NicInfo {
    let device_path = Path::new("/sys/class/net").join(interface).join("device");
    let uevent = fs::read_to_string(device_path.join("uevent")).ok();
    let pci_slot = uevent
        .as_deref()
        .and_then(|content| uevent_value(content, "PCI_SLOT_NAME"))
        .or_else(|| pci_slot_from_device_path(&device_path));

    NicInfo {
        model: pci_slot.as_deref().and_then(resolve_pci_model),
        driver: uevent
            .as_deref()
            .and_then(|content| uevent_value(content, "DRIVER"))
            .or_else(|| driver_from_device_path(&device_path)),
        pci_slot,
        pci_vendor: read_trimmed_file(&device_path.join("vendor")),
        pci_device: read_trimmed_file(&device_path.join("device")),
        pci_subsystem_vendor: read_trimmed_file(&device_path.join("subsystem_vendor")),
        pci_subsystem_device: read_trimmed_file(&device_path.join("subsystem_device")),
    }
}

fn read_trimmed_file(path: &Path) -> Option<String> {
    let value = fs::read_to_string(path).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn uevent_value(content: &str, key: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let (line_key, value) = line.split_once('=')?;
        (line_key == key && !value.is_empty()).then(|| value.to_owned())
    })
}

fn pci_slot_from_device_path(device_path: &Path) -> Option<String> {
    let path = fs::canonicalize(device_path).ok()?;
    let slot = path.file_name()?.to_str()?;
    looks_like_pci_slot(slot).then(|| slot.to_owned())
}

fn looks_like_pci_slot(value: &str) -> bool {
    value.contains(':')
        && value.contains('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
}

fn driver_from_device_path(device_path: &Path) -> Option<String> {
    let path = fs::read_link(device_path.join("driver")).ok()?;
    path.file_name()?.to_str().map(ToOwned::to_owned)
}

fn resolve_pci_model(pci_slot: &str) -> Option<String> {
    let output = Command::new("lspci")
        .arg("-s")
        .arg(pci_slot)
        .arg("-nn")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_lspci_model(&stdout)
}

fn parse_lspci_model(output: &str) -> Option<String> {
    let line = output.lines().find(|line| !line.trim().is_empty())?.trim();
    let (_, model) = line.split_once(": ")?;
    let model = model
        .rsplit_once(" (rev ")
        .map_or(model, |(model, _)| model)
        .trim();
    (!model.is_empty()).then(|| model.to_owned())
}

fn normalize_pci_id(value: &str) -> String {
    value
        .trim()
        .strip_prefix("0x")
        .unwrap_or_else(|| value.trim())
        .to_ascii_lowercase()
}

fn check_ring_size(name: RingName, size: usize) -> RingCheck {
    let valid = size != 0 && size.is_power_of_two();
    RingCheck {
        name,
        size,
        valid,
        recommended_size: (!valid)
            .then(|| next_power_of_two_recommendation(size))
            .flatten(),
    }
}

fn render_invalid_tx_ring_failure(interface: &str, check: &RingCheck) -> String {
    if let Some(size) = check.recommended_size {
        format!(
            "TX ring size is invalid: {} (try: sudo ethtool -G {interface} tx {size})",
            check.size
        )
    } else {
        format!(
            "TX ring size is invalid: {} (try: sudo ethtool -G {interface} tx <power-of-two-size>)",
            check.size
        )
    }
}

fn render_report(report: &InfoReport, verbose: bool) -> String {
    let mut output = String::new();
    output.push_str(&format!("Interface: {}\n", report.interface));
    render_nic_info(&mut output, &report.nic_info);
    output.push('\n');
    output.push_str(&format!(
        "{} XDP: {}\n",
        if xdp_supported(&report.query) {
            CHECK
        } else {
            CROSS
        },
        if xdp_supported(&report.query) {
            "supported"
        } else {
            "not supported"
        }
    ));
    output.push_str(&format!(
        "{} Zero-copy: {}\n",
        if zero_copy_supported(&report.query) {
            CHECK
        } else {
            CROSS
        },
        if zero_copy_supported(&report.query) {
            "supported"
        } else {
            "not supported"
        }
    ));
    render_ring_report(&mut output, &report.interface, &report.ring_report, verbose);

    if verbose {
        output.push('\n');
        render_verbose(&mut output, report);
    }

    output
}

fn render_interface_reports(reports: &[InterfaceInfoReport], verbose: bool) -> String {
    let mut output = String::new();
    render_interface_report_group(&mut output, reports, InterfaceKind::Physical, verbose);
    render_interface_report_group(&mut output, reports, InterfaceKind::VirtualLogical, verbose);
    output
}

fn render_interface_report_group(
    output: &mut String,
    reports: &[InterfaceInfoReport],
    kind: InterfaceKind,
    verbose: bool,
) {
    let reports = reports
        .iter()
        .filter(|report| report.kind() == kind)
        .collect::<Vec<_>>();
    if reports.is_empty() {
        return;
    }

    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(kind.heading());
    output.push('\n');

    for (index, report) in reports.iter().enumerate() {
        if verbose && index > 0 {
            output.push('\n');
        }

        match (verbose, *report) {
            (true, InterfaceInfoReport::Report { report, .. }) => {
                output.push_str(&render_report(report, true));
            }
            (true, InterfaceInfoReport::Error { report, .. }) => {
                render_error_report(output, report);
            }
            (false, InterfaceInfoReport::Report { report, .. }) => {
                render_summary_report(output, report);
            }
            (false, InterfaceInfoReport::Error { report, .. }) => {
                render_summary_error_report(output, report);
            }
        }
    }
}

fn render_summary_report(output: &mut String, report: &InfoReport) {
    output.push_str(&format!(
        "{}: {}, {}, {}, {}\n",
        report.interface,
        render_summary_status("XDP", xdp_supported(&report.query)),
        render_summary_status("zero-copy", zero_copy_supported(&report.query)),
        render_summary_tx_ring(&report.ring_report),
        report
            .nic_info
            .display_model()
            .unwrap_or_else(|| "NIC unavailable".to_string())
    ));
}

fn render_summary_error_report(output: &mut String, report: &InfoErrorReport) {
    output.push_str(&format!(
        "{}: {CROSS} error: {}\n",
        report.interface, report.error
    ));
}

fn render_summary_status(label: &str, ready: bool) -> String {
    format!("{} {label}", if ready { CHECK } else { CROSS })
}

fn render_summary_tx_ring(report: &RingReport) -> String {
    match report {
        RingReport::Sizes { tx, .. } => format!(
            "{} TX ring {}",
            if tx.valid { CHECK } else { CROSS },
            tx.size
        ),
        RingReport::Unavailable(_) => format!("{WARNING} TX ring unknown"),
    }
}

fn render_error_report(output: &mut String, report: &InfoErrorReport) {
    output.push_str(&format!("Interface: {}\n", report.interface));
    render_nic_info(output, &report.nic_info);
    output.push('\n');
    output.push_str(&format!("{CROSS} Error: {}\n", report.error));
}

fn all_interface_reports_ready(reports: &[InterfaceInfoReport]) -> bool {
    reports.iter().all(InterfaceInfoReport::ready)
}

fn render_nic_info(output: &mut String, nic_info: &NicInfo) {
    if let Some(model) = nic_info.display_model() {
        output.push_str(&format!("NIC: {model}\n"));
    } else {
        output.push_str("NIC: unavailable\n");
    }

    if let Some(driver) = &nic_info.driver {
        output.push_str(&format!("Driver: {driver}\n"));
    }

    if let Some(pci_slot) = &nic_info.pci_slot {
        output.push_str(&format!("PCI slot: {pci_slot}\n"));
    }
}

fn render_ring_report(output: &mut String, interface: &str, report: &RingReport, verbose: bool) {
    match report {
        RingReport::Sizes { tx, rx } => {
            render_ring_check(output, interface, tx);
            if verbose {
                render_informational_rx_ring_check(output, interface, rx);
            }
        }
        RingReport::Unavailable(err) => {
            output.push_str(&format!(
                "{WARNING}  Ring sizes: unable to query with ethtool ioctl: {err}\n"
            ));
        }
    }
}

fn render_informational_rx_ring_check(output: &mut String, interface: &str, check: &RingCheck) {
    output.push_str(&format!(
        "{} {} ring: {}, informational only\n",
        if check.valid { CHECK } else { WARNING },
        check.name.upper(),
        check.size
    ));
    if check.valid {
        return;
    }

    if let Some(size) = check.recommended_size {
        output.push_str(&format!(
            "   To fix it you can try, for example: sudo ethtool -G {interface} {} {size}\n",
            check.name.ethtool_arg()
        ));
    } else {
        output.push_str(&format!(
            "   To fix it you can try, for example: sudo ethtool -G {interface} {} <power-of-two-size>\n",
            check.name.ethtool_arg()
        ));
    }
}

fn render_ring_check(output: &mut String, interface: &str, check: &RingCheck) {
    if check.valid {
        output.push_str(&format!(
            "{CHECK} {} ring: {}, ok\n",
            check.name.upper(),
            check.size
        ));
        return;
    }

    output.push_str(&format!(
        "{CROSS} {} ring: {}, invalid\n",
        check.name.upper(),
        check.size
    ));
    if let Some(size) = check.recommended_size {
        output.push_str(&format!(
            "   To fix it you can try, for example: sudo ethtool -G {interface} {} {size}\n",
            check.name.ethtool_arg()
        ));
    } else {
        output.push_str(&format!(
            "   To fix it you can try, for example: sudo ethtool -G {interface} {} <power-of-two-size>\n",
            check.name.ethtool_arg()
        ));
    }
}

fn render_verbose(output: &mut String, report: &InfoReport) {
    let query = &report.query;
    output.push_str("Verbose:\n");
    output.push_str(&format!("  ifindex={}\n", query.ifindex));
    render_verbose_nic_info(output, &report.nic_info);
    output.push_str(&format!("  xdp_features=0x{:016x}\n", query.xdp_features));
    output.push_str(&format!(
        "  xdp_zc_max_segs={}\n",
        query
            .xdp_zc_max_segs
            .map_or_else(|| "n/a".to_owned(), |value| value.to_string())
    ));
    output.push_str(&format!(
        "  xdp_rx_metadata_features=0x{:016x}\n",
        query.xdp_rx_metadata_features.unwrap_or(0)
    ));
    output.push_str(&format!(
        "  xsk_features=0x{:016x}\n",
        query.xsk_features.unwrap_or(0)
    ));

    for feature in XdpFeature::ALL {
        if feature == XdpFeature::ZcMaxSegs {
            output.push_str(&format!(
                "  {}: {}\n",
                feature.as_str(),
                query
                    .xdp_zc_max_segs
                    .map_or_else(|| "n/a".to_owned(), |value| value.to_string())
            ));
        } else {
            output.push_str(&format!(
                "  {}: {}\n",
                feature.as_str(),
                if feature_supported(query, feature) {
                    "yes"
                } else {
                    "no"
                }
            ));
        }
    }
}

fn render_verbose_nic_info(output: &mut String, nic_info: &NicInfo) {
    render_verbose_optional(output, "pci_vendor", nic_info.pci_vendor.as_deref());
    render_verbose_optional(output, "pci_device", nic_info.pci_device.as_deref());
    render_verbose_optional(
        output,
        "pci_subsystem_vendor",
        nic_info.pci_subsystem_vendor.as_deref(),
    );
    render_verbose_optional(
        output,
        "pci_subsystem_device",
        nic_info.pci_subsystem_device.as_deref(),
    );
}

fn render_verbose_optional(output: &mut String, name: &str, value: Option<&str>) {
    output.push_str(&format!("  {name}={}\n", value.unwrap_or("n/a")));
}

fn xdp_supported(query: &XdpFeatureQuery) -> bool {
    feature_supported(query, XdpFeature::Basic) && feature_supported(query, XdpFeature::Redirect)
}

fn zero_copy_supported(query: &XdpFeatureQuery) -> bool {
    feature_supported(query, XdpFeature::XskZerocopy)
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

fn next_power_of_two_recommendation(value: usize) -> Option<usize> {
    if value == 0 {
        None
    } else {
        value.checked_next_power_of_two()
    }
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
pub enum QueryError {
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

    fn query_with_features(xdp_features: u64) -> XdpFeatureQuery {
        XdpFeatureQuery {
            ifindex: 7,
            xdp_features,
            xdp_zc_max_segs: Some(1),
            xdp_rx_metadata_features: Some(0),
            xsk_features: Some(0),
        }
    }

    fn report_with_nic_info(nic_info: NicInfo) -> InfoReport {
        report_with_interface(
            "eth0",
            NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT | NETDEV_XDP_ACT_XSK_ZEROCOPY,
            1024,
            nic_info,
        )
    }

    fn report_with_interface(
        interface: &str,
        xdp_features: u64,
        tx_ring_size: usize,
        nic_info: NicInfo,
    ) -> InfoReport {
        InfoReport {
            interface: interface.to_string(),
            nic_info,
            query: query_with_features(xdp_features),
            ring_report: RingReport::Sizes {
                tx: check_ring_size(RingName::Tx, tx_ring_size),
                rx: check_ring_size(RingName::Rx, 1024),
            },
        }
    }

    fn check_report(
        xdp_features: u64,
        tx_ring_size: usize,
        require_zero_copy: bool,
    ) -> CheckReport {
        CheckReport {
            interface: "eth0".to_string(),
            query: query_with_features(xdp_features),
            ring_report: RingReport::Sizes {
                tx: check_ring_size(RingName::Tx, tx_ring_size),
                rx: check_ring_size(RingName::Rx, 1024),
            },
            require_zero_copy,
        }
    }

    fn physical_report(report: InfoReport) -> InterfaceInfoReport {
        InterfaceInfoReport::Report {
            kind: InterfaceKind::Physical,
            report,
        }
    }

    fn virtual_report(report: InfoReport) -> InterfaceInfoReport {
        InterfaceInfoReport::Report {
            kind: InterfaceKind::VirtualLogical,
            report,
        }
    }

    fn virtual_error_report(report: InfoErrorReport) -> InterfaceInfoReport {
        InterfaceInfoReport::Error {
            kind: InterfaceKind::VirtualLogical,
            report,
        }
    }

    #[test]
    fn classifies_xdp_and_zero_copy_support() {
        let unsupported = query_with_features(0);
        assert!(!xdp_supported(&unsupported));
        assert!(!zero_copy_supported(&unsupported));

        let copy_mode = query_with_features(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT);
        assert!(xdp_supported(&copy_mode));
        assert!(!zero_copy_supported(&copy_mode));

        let zero_copy = query_with_features(
            NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT | NETDEV_XDP_ACT_XSK_ZEROCOPY,
        );
        assert!(xdp_supported(&zero_copy));
        assert!(zero_copy_supported(&zero_copy));
    }

    #[test]
    fn render_default_output_uses_status_icons() {
        let report = InfoReport {
            interface: "eth0".to_string(),
            nic_info: NicInfo::default(),
            query: query_with_features(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT),
            ring_report: RingReport::Sizes {
                tx: check_ring_size(RingName::Tx, 511),
                rx: check_ring_size(RingName::Rx, 512),
            },
        };

        let output = render_report(&report, false);
        assert!(output.contains("NIC: unavailable"));
        assert!(output.contains("✅ XDP: supported"));
        assert!(output.contains("❌ Zero-copy: not supported"));
        assert!(output.contains("❌ TX ring: 511, invalid"));
        assert!(output.contains("To fix it you can try, for example: sudo ethtool -G eth0 tx 512"));
        assert!(!output.contains("RX ring"));
    }

    #[test]
    fn render_default_output_includes_nic_model() {
        let output = render_report(
            &report_with_nic_info(NicInfo {
                model: Some(
                    "Intel Corporation Ethernet Controller X710 for 10GBASE-T [8086:15ff]"
                        .to_string(),
                ),
                driver: Some("i40e".to_string()),
                pci_slot: Some("0000:05:00.0".to_string()),
                ..NicInfo::default()
            }),
            false,
        );

        assert!(
            output.contains(
                "NIC: Intel Corporation Ethernet Controller X710 for 10GBASE-T [8086:15ff]"
            )
        );
        assert!(output.contains("Driver: i40e"));
        assert!(output.contains("PCI slot: 0000:05:00.0"));
    }

    #[test]
    fn render_default_output_falls_back_to_pci_ids() {
        let output = render_report(
            &report_with_nic_info(NicInfo {
                pci_vendor: Some("0x8086".to_string()),
                pci_device: Some("0x15FF".to_string()),
                ..NicInfo::default()
            }),
            false,
        );

        assert!(output.contains("NIC: PCI device [8086:15ff]"));
    }

    #[test]
    fn render_verbose_output_includes_known_flags() {
        let report = InfoReport {
            interface: "eth0".to_string(),
            nic_info: NicInfo {
                pci_vendor: Some("0x8086".to_string()),
                pci_device: Some("0x15ff".to_string()),
                pci_subsystem_vendor: Some("0x15d9".to_string()),
                pci_subsystem_device: Some("0x1c27".to_string()),
                ..NicInfo::default()
            },
            query: query_with_features(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_XSK_ZEROCOPY),
            ring_report: RingReport::Sizes {
                tx: check_ring_size(RingName::Tx, 1024),
                rx: check_ring_size(RingName::Rx, 511),
            },
        };

        let output = render_report(&report, true);
        assert!(output.contains("⚠️ RX ring: 511, informational only"));
        assert!(output.contains("To fix it you can try, for example: sudo ethtool -G eth0 rx 512"));
        assert!(output.contains("Verbose:"));
        assert!(output.contains("pci_vendor=0x8086"));
        assert!(output.contains("pci_device=0x15ff"));
        assert!(output.contains("pci_subsystem_vendor=0x15d9"));
        assert!(output.contains("pci_subsystem_device=0x1c27"));
        assert!(output.contains("xdp_features=0x"));
        assert!(output.contains("NETDEV_XDP_ACT_BASIC: yes"));
        assert!(output.contains("NETDEV_XDP_ACT_REDIRECT: no"));
        assert!(output.contains("NETDEV_A_DEV_XDP_ZC_MAX_SEGS: 1"));
    }

    #[test]
    fn parse_lspci_model_extracts_model_name() {
        let output = "05:00.0 Ethernet controller [0200]: Intel Corporation Ethernet Controller X710 for 10GBASE-T [8086:15ff] (rev 02)\n";

        assert_eq!(
            parse_lspci_model(output),
            Some(
                "Intel Corporation Ethernet Controller X710 for 10GBASE-T [8086:15ff]".to_string()
            )
        );
    }

    #[test]
    fn classify_pci_backed_interface_as_physical() {
        assert_eq!(
            classify_canonical_device_path(Path::new(
                "/sys/devices/pci0000:00/0000:00:03.1/0000:05:00.0"
            )),
            InterfaceKind::Physical
        );
    }

    #[test]
    fn classify_virtual_interface_as_virtual_logical() {
        assert_eq!(
            classify_canonical_device_path(Path::new("/sys/devices/virtual/net/lo/device")),
            InterfaceKind::VirtualLogical
        );
    }

    #[test]
    fn classify_missing_or_unknown_interface_as_virtual_logical() {
        assert_eq!(
            classify_interface_device_path(Path::new(
                "/tmp/af-xdp-client-missing-interface-device"
            )),
            InterfaceKind::VirtualLogical
        );
        assert_eq!(
            classify_canonical_device_path(Path::new("/sys/devices/platform/example-netdev")),
            InterfaceKind::VirtualLogical
        );
    }

    #[test]
    fn render_all_summary_output_includes_one_line_per_interface() {
        let reports = vec![
            physical_report(report_with_nic_info(NicInfo {
                pci_vendor: Some("0x8086".to_string()),
                pci_device: Some("0x15ff".to_string()),
                ..NicInfo::default()
            })),
            virtual_report(report_with_interface(
                "eth1",
                NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT,
                511,
                NicInfo::default(),
            )),
        ];

        let output = render_interface_reports(&reports, false);

        assert!(output.contains("Physical interfaces:\n"));
        assert!(output.contains("\nVirtual/logical interfaces:\n"));
        assert!(
            output
                .contains("eth0: ✅ XDP, ✅ zero-copy, ✅ TX ring 1024, PCI device [8086:15ff]\n")
        );
        assert!(output.contains("eth1: ✅ XDP, ❌ zero-copy, ❌ TX ring 511, NIC unavailable\n"));
        assert!(!all_interface_reports_ready(&reports));
    }

    #[test]
    fn render_all_summary_output_includes_query_errors() {
        let reports = vec![virtual_error_report(InfoErrorReport {
            interface: "lo".to_string(),
            nic_info: NicInfo::default(),
            error: "netlink response did not include NETDEV_A_DEV_XDP_FEATURES".to_string(),
        })];

        let output = render_interface_reports(&reports, false);

        assert_eq!(
            output,
            "Virtual/logical interfaces:\nlo: ❌ error: netlink response did not include NETDEV_A_DEV_XDP_FEATURES\n"
        );
        assert!(!all_interface_reports_ready(&reports));
    }

    #[test]
    fn render_all_verbose_output_uses_detailed_blocks() {
        let reports = vec![
            physical_report(report_with_interface(
                "eth0",
                NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT | NETDEV_XDP_ACT_XSK_ZEROCOPY,
                1024,
                NicInfo::default(),
            )),
            virtual_error_report(InfoErrorReport {
                interface: "lo".to_string(),
                nic_info: NicInfo::default(),
                error: "boom".to_string(),
            }),
        ];

        let output = render_interface_reports(&reports, true);

        assert!(output.contains("Physical interfaces:\nInterface: eth0"));
        assert!(output.contains("Interface: eth0\nNIC: unavailable\n\n✅ XDP: supported"));
        assert!(output.contains("Verbose:"));
        assert!(output.contains("\nVirtual/logical interfaces:\nInterface: lo"));
        assert!(output.contains("Interface: lo\nNIC: unavailable\n\n❌ Error: boom\n"));
    }

    #[test]
    fn ring_size_check_accepts_powers_of_two() {
        let check = check_ring_size(RingName::Tx, 1024);
        assert!(check.valid);
        assert_eq!(check.recommended_size, None);
    }

    #[test]
    fn ring_size_check_rejects_zero_and_non_powers_of_two() {
        let zero = check_ring_size(RingName::Tx, 0);
        assert!(!zero.valid);
        assert_eq!(zero.recommended_size, None);

        let non_power = check_ring_size(RingName::Tx, 511);
        assert!(!non_power.valid);
        assert_eq!(non_power.recommended_size, Some(512));
    }

    #[test]
    fn next_power_of_two_recommendations_are_correct() {
        assert_eq!(next_power_of_two_recommendation(511), Some(512));
        assert_eq!(next_power_of_two_recommendation(3000), Some(4096));
        assert_eq!(next_power_of_two_recommendation(0), None);
    }

    #[test]
    fn ring_report_ready_requires_valid_known_sizes() {
        let ready = RingReport::Sizes {
            tx: check_ring_size(RingName::Tx, 1024),
            rx: check_ring_size(RingName::Rx, 511),
        };
        assert!(ready.ready());

        let not_ready = RingReport::Sizes {
            tx: check_ring_size(RingName::Tx, 511),
            rx: check_ring_size(RingName::Rx, 1024),
        };
        assert!(!not_ready.ready());
    }

    #[test]
    fn report_ready_requires_xdp_zero_copy_and_valid_rings() {
        let report = InfoReport {
            interface: "eth0".to_string(),
            nic_info: NicInfo::default(),
            query: query_with_features(
                NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT | NETDEV_XDP_ACT_XSK_ZEROCOPY,
            ),
            ring_report: RingReport::Sizes {
                tx: check_ring_size(RingName::Tx, 1024),
                rx: check_ring_size(RingName::Rx, 511),
            },
        };
        assert!(report.ready());

        let report = InfoReport {
            interface: "eth0".to_string(),
            nic_info: NicInfo::default(),
            query: query_with_features(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT),
            ring_report: RingReport::Sizes {
                tx: check_ring_size(RingName::Tx, 1024),
                rx: check_ring_size(RingName::Rx, 512),
            },
        };
        assert!(!report.ready());
    }

    #[test]
    fn check_passes_with_xdp_and_valid_tx_ring() {
        let report = check_report(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT, 1024, false);

        assert!(report.failures().is_empty());
    }

    #[test]
    fn check_fails_without_xdp_support() {
        let report = check_report(0, 1024, false);

        assert_eq!(report.failures(), vec!["XDP not supported"]);
    }

    #[test]
    fn check_fails_invalid_tx_ring() {
        let report = check_report(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT, 511, false);

        assert_eq!(
            report.failures(),
            vec!["TX ring size is invalid: 511 (try: sudo ethtool -G eth0 tx 512)"]
        );
    }

    #[test]
    fn check_fails_when_ring_query_is_unavailable() {
        let report = CheckReport {
            interface: "eth0".to_string(),
            query: query_with_features(NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT),
            ring_report: RingReport::Unavailable(std::io::Error::other("ring query failed")),
            require_zero_copy: false,
        };

        assert_eq!(
            report.failures(),
            vec!["unable to query ring sizes: ring query failed"]
        );
    }

    #[test]
    fn check_zero_copy_is_optional_but_enforced_when_requested() {
        let copy_mode = NETDEV_XDP_ACT_BASIC | NETDEV_XDP_ACT_REDIRECT;
        assert!(check_report(copy_mode, 1024, false).failures().is_empty());
        assert_eq!(
            check_report(copy_mode, 1024, true).failures(),
            vec!["zero-copy not supported"]
        );

        let zero_copy = copy_mode | NETDEV_XDP_ACT_XSK_ZEROCOPY;
        assert!(check_report(zero_copy, 1024, true).failures().is_empty());
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
}
