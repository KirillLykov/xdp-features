# xdp-features

`xdp-features` is a Linux command-line tool for checking which XDP-related
capabilities a network interface reports through the kernel `netdev` generic
netlink family.

It queries the selected interface and prints whether each requested capability
is available. The exit status is intended for shell scripts and CI checks.

## Usage

```text
xdp-features [OPTIONS] [COMMAND]
```

Options:

```text
-i, --interface <IFNAME>  Network interface to inspect
-v, --verbose...          Increase diagnostic verbosity
-h, --help                Print help
-V, --version             Print version
```

The interface option is required for queries:

```sh
target/debug/xdp-features --interface eth0
```

When no subcommand is provided, the tool prints every known capability.

## Checking Specific Features

Use the `features` subcommand to check only selected capabilities:

```sh
target/debug/xdp-features --interface eth0 features \
  NETDEV_XDP_ACT_BASIC \
  NETDEV_XDP_ACT_REDIRECT \
  NETDEV_XDP_ACT_XSK_ZEROCOPY
```

`--interface` is global, so this is equivalent:

```sh
target/debug/xdp-features features NETDEV_XDP_ACT_BASIC --interface eth0
```

The `features` subcommand requires at least one feature name.

## Output

Default mode prints all known capabilities:

```text
NETDEV_XDP_ACT_BASIC: yes
NETDEV_XDP_ACT_REDIRECT: yes
NETDEV_XDP_ACT_NDO_XMIT: no
NETDEV_XDP_ACT_XSK_ZEROCOPY: no
NETDEV_XDP_ACT_HW_OFFLOAD: no
NETDEV_XDP_ACT_RX_SG: yes
NETDEV_XDP_ACT_NDO_XMIT_SG: no
NETDEV_A_DEV_XDP_ZC_MAX_SEGS: 4
NETDEV_XDP_RX_METADATA_TIMESTAMP: yes
NETDEV_XDP_RX_METADATA_HASH: no
NETDEV_XDP_RX_METADATA_VLAN_TAG: no
NETDEV_XSK_FLAGS_TX_TIMESTAMP: no
NETDEV_XSK_FLAGS_TX_CHECKSUM: yes
```

With `--verbose`, the raw netlink values are also printed:

```sh
target/debug/xdp-features --interface eth0 --verbose
```

## Exit Status

```text
0  all selected capabilities are supported or present
1  at least one selected capability is unsupported or missing
2  query/runtime error
```

Example shell check:

```sh
if target/debug/xdp-features --interface eth0 features NETDEV_XDP_ACT_BASIC; then
  echo "supported"
else
  case $? in
    1) echo "not supported" ;;
    2) echo "query failed" ;;
  esac
fi
```

