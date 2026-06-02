# xdp-features

`xdp-features` is a Linux command-line tool for inspecting and checking
XDP/AF_XDP readiness on network interfaces.

It has two subcommands:

- `info` prints a human-readable report.
- `check` is quiet and intended for scripts.

## Usage

```text
xdp-features <COMMAND>
```

### Human Report

Inspect every interface:

```sh
target/debug/xdp-features info
```

Inspect one interface:

```sh
target/debug/xdp-features info --interface eth0
target/debug/xdp-features info -i eth0 --verbose
```

Without `--interface`, `info` prints a compact summary for every interface,
grouped as physical interfaces and virtual/logical interfaces. With
`--interface`, it prints a detailed report for that interface.

Default output shows NIC identity when available and status lines for XDP,
zero-copy, and TX ring size. Verbose mode also prints raw kernel netlink values,
all known feature flags, PCI details, and RX ring size as informational output.

Example summary:

```text
Physical interfaces:
enp5s0f0: ✅ XDP, ✅ zero-copy, ✅ TX ring 512, Intel Corporation Ethernet Controller X710 for 10GBASE-T [8086:15ff]

Virtual/logical interfaces:
lo: ❌ error: netlink response did not include NETDEV_A_DEV_XDP_FEATURES
```

Example detailed report:

```text
Interface: enp5s0f0
NIC: Intel Corporation Ethernet Controller X710 for 10GBASE-T [8086:15ff]
Driver: i40e
PCI slot: 0000:05:00.0

✅ XDP: supported
✅ Zero-copy: supported
✅ TX ring: 512, ok
```

NIC identity is best-effort. The tool reads PCI information from sysfs and uses
`lspci` when available to resolve a human-readable model. If the model cannot be
resolved, it falls back to PCI vendor/device IDs.

### Script Check

Check that an interface supports XDP and has a valid TX ring size. `check`
always requires `-i` or `--interface`:

```sh
target/debug/xdp-features check --interface eth0
```

Also require AF_XDP zero-copy support:

```sh
target/debug/xdp-features check -i eth0 --zero-copy
```

`check` prints nothing on success. On failure it writes only the failed
requirement or runtime error to stderr and exits nonzero. Possible failure
messages include:

```text
XDP not supported
zero-copy not supported
TX ring size is invalid: 511 (try: sudo ethtool -G eth0 tx 512)
unable to query ring sizes: <error>
error: <netlink or interface error>
```

## Exit Status

`info`:

```text
0  inspected interface reports are ready
1  inspected interface reports were queried but are not ready
2  runtime/query error
```

`check`:

```text
0  all requested checks passed
1  one or more checks failed, including runtime/query errors
```

For `check`, the default checks are XDP support and valid TX ring size.
`--zero-copy` additionally requires zero-copy support. A TX ring size of zero or
a non-power-of-two TX ring size fails the check. Failure to query ring sizes also
fails the check.
