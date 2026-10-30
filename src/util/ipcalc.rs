//! IPv4/IPv6 CIDR arithmetic for the `/tool ipcalc` subcommand.
//!
//! Host bits in the input are accepted and normalised: the report always shows
//! the network address and flags that host bits were cleared. Address counts are
//! computed in `u128`, so IPv6 prefixes down to `/0` never overflow.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use anyhow::{Result, bail};

/// `2^128`, the address count of `::/0`; `1u128 << 128` would overflow.
const V6_TOTAL_FOR_SLASH_ZERO: &str = "340282366920938463463374607431768211456";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::V4 => "IPv4",
            Family::V6 => "IPv6",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Report {
    pub family: Family,
    pub input: String,
    pub prefix: u8,
    pub network: String,
    pub netmask: Option<String>,
    pub broadcast: Option<String>,
    pub first: String,
    pub last: String,
    pub total: String,
    pub usable: String,
    pub scope: &'static str,
    pub classification: &'static str,
    pub normalized: bool,
}

impl Report {
    /// `network/prefix`, the canonical form of the input block.
    pub fn cidr(&self) -> String {
        format!("{}/{}", self.network, self.prefix)
    }

    /// Aligned key/value lines for a monospace code block.
    pub fn lines(&self) -> String {
        let mut rows: Vec<(&str, String)> = vec![
            ("Input", self.input.clone()),
            ("Prefix", format!("/{}", self.prefix)),
            ("Network", self.cidr()),
        ];
        if let Some(netmask) = &self.netmask {
            rows.push(("Netmask", netmask.clone()));
        }
        if let Some(broadcast) = &self.broadcast {
            rows.push(("Broadcast", broadcast.clone()));
        }
        rows.push(("First usable", self.first.clone()));
        rows.push(("Last usable", self.last.clone()));
        rows.push(("Total addrs", self.total.clone()));
        rows.push(("Usable hosts", self.usable.clone()));
        rows.push(("Scope", self.scope.to_owned()));
        rows.push(("Classification", self.classification.to_owned()));
        rows.push((
            "Host bits",
            if self.normalized {
                "cleared (network shown)".to_owned()
            } else {
                "none set".to_owned()
            },
        ));

        rows.iter()
            .map(|(label, value)| format!("{label:<15}{value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Parse an IPv4 or IPv6 CIDR block. A bare address is treated as `/32` or `/128`.
pub fn parse(input: &str) -> Result<Report> {
    let raw = input.trim();
    if raw.is_empty() {
        bail!("Give a CIDR block, e.g. 10.0.0.0/24 or 2001:db8::/32.");
    }
    if raw.chars().count() > 50 {
        bail!("CIDR must be 50 characters or fewer.");
    }

    let (address, prefix) = match raw.split_once('/') {
        Some((address, prefix)) => (address.trim(), Some(prefix.trim())),
        None => (raw, None),
    };
    if address.is_empty() {
        bail!("`{raw}` has no address before the prefix.");
    }

    if address.contains(':') {
        parse_v6(raw, address, prefix)
    } else {
        parse_v4(raw, address, prefix)
    }
}

fn parse_v4(input: &str, address: &str, prefix_raw: Option<&str>) -> Result<Report> {
    let address = Ipv4Addr::from_str(address)
        .map_err(|_| anyhow::anyhow!("`{address}` is not a valid IPv4 address."))?;
    let ip = u32::from(address);
    let prefix = prefix_or(prefix_raw, 32)?;
    if prefix > 32 {
        bail!("IPv4 prefixes range from /0 to /32 (got /{prefix}).");
    }

    let mask = v4_mask(prefix);
    let network = ip & mask;
    let broadcast = network | !mask;
    let total: u128 = 1u128 << (32 - prefix);
    let (first, last, usable) = match prefix {
        32 => (network, network, 1u128),
        31 => (network, broadcast, 2u128),
        _ => (network + 1, broadcast - 1, total - 2),
    };

    Ok(Report {
        family: Family::V4,
        input: input.to_owned(),
        prefix,
        network: Ipv4Addr::from(network).to_string(),
        netmask: Some(Ipv4Addr::from(mask).to_string()),
        broadcast: Some(Ipv4Addr::from(broadcast).to_string()),
        first: Ipv4Addr::from(first).to_string(),
        last: Ipv4Addr::from(last).to_string(),
        total: group(total),
        usable: group(usable),
        scope: v4_scope(network),
        classification: v4_classification(network),
        normalized: network != ip,
    })
}

fn parse_v6(input: &str, address: &str, prefix_raw: Option<&str>) -> Result<Report> {
    let address = Ipv6Addr::from_str(address)
        .map_err(|_| anyhow::anyhow!("`{address}` is not a valid IPv6 address."))?;
    let ip = u128::from(address);
    let prefix = prefix_or(prefix_raw, 128)?;
    if prefix > 128 {
        bail!("IPv6 prefixes range from /0 to /128 (got /{prefix}).");
    }

    let mask = v6_mask(prefix);
    let network = ip & mask;
    let last = network | !mask;
    let total = v6_total(prefix);

    Ok(Report {
        family: Family::V6,
        input: input.to_owned(),
        prefix,
        network: Ipv6Addr::from(network).to_string(),
        netmask: None,
        broadcast: None,
        first: Ipv6Addr::from(network).to_string(),
        last: Ipv6Addr::from(last).to_string(),
        usable: total.clone(),
        total,
        scope: v6_scope(network),
        classification: v6_classification(network),
        normalized: network != ip,
    })
}

fn prefix_or(raw: Option<&str>, default: u8) -> Result<u8> {
    match raw {
        None => Ok(default),
        Some(value) => value
            .parse::<u8>()
            .map_err(|_| anyhow::anyhow!("`{value}` is not a valid prefix length.")),
    }
}

fn v4_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    }
}

fn v6_mask(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    }
}

fn v6_total(prefix: u8) -> String {
    if prefix == 0 {
        group_digits(V6_TOTAL_FOR_SLASH_ZERO)
    } else {
        group(1u128 << (128 - prefix))
    }
}

fn group(value: u128) -> String {
    group_digits(&value.to_string())
}

/// `18446744073709551616` → `18,446,744,073,709,551,616`.
fn group_digits(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.char_indices() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn is_private_v4(ip: u32) -> bool {
    let octets = ip.to_be_bytes();
    octets[0] == 10
        || (octets[0] == 172 && (16..=31).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 168)
}

fn v4_scope(ip: u32) -> &'static str {
    let octets = ip.to_be_bytes();
    if octets[0] == 127 {
        "loopback"
    } else if octets[0] == 169 && octets[1] == 254 {
        "link-local"
    } else if (224..=239).contains(&octets[0]) {
        "multicast"
    } else if is_private_v4(ip) {
        "private"
    } else {
        "global"
    }
}

fn v4_classification(ip: u32) -> &'static str {
    let octets = ip.to_be_bytes();
    if octets[0] == 127 {
        "loopback (RFC 1122)"
    } else if octets[0] == 169 && octets[1] == 254 {
        "link-local (RFC 3927)"
    } else if (224..=239).contains(&octets[0]) {
        "multicast (RFC 5771)"
    } else if ip == u32::MAX {
        "limited broadcast (RFC 919)"
    } else if octets[0] == 0 {
        "reserved — this network (RFC 1122)"
    } else if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        "shared — CGNAT (RFC 6598)"
    } else if is_private_v4(ip) {
        "private (RFC 1918)"
    } else if (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
    {
        "documentation (RFC 5737)"
    } else if octets[0] == 198 && matches!(octets[1], 18 | 19) {
        "benchmarking (RFC 2544)"
    } else if octets[0] >= 240 {
        "reserved (RFC 1112)"
    } else {
        "global (public)"
    }
}

fn v6_link_local(ip: u128) -> bool {
    ip >> 118 == 0x3fa
}

fn v6_multicast(ip: u128) -> bool {
    ip >> 120 == 0xff
}

fn v6_unique_local(ip: u128) -> bool {
    let top7 = ip >> 121;
    top7 == 0x7e || top7 == 0x7f
}

fn v6_scope(ip: u128) -> &'static str {
    if ip == 1 {
        "loopback"
    } else if v6_link_local(ip) {
        "link-local"
    } else if v6_multicast(ip) {
        "multicast"
    } else if v6_unique_local(ip) {
        "private"
    } else {
        "global"
    }
}

fn v6_classification(ip: u128) -> &'static str {
    if ip == 0 {
        "unspecified (RFC 4291)"
    } else if ip == 1 {
        "loopback (RFC 4291)"
    } else if v6_link_local(ip) {
        "link-local (RFC 4291)"
    } else if v6_multicast(ip) {
        "multicast (RFC 4291)"
    } else if v6_unique_local(ip) {
        "unique-local (RFC 4193)"
    } else if ip >> 96 == 0x2001_0db8 {
        "documentation (RFC 3849)"
    } else if ip >> 125 == 0b001 {
        "global (public)"
    } else if ip >> 32 == 0xffff {
        "IPv4-mapped (RFC 4291)"
    } else {
        "reserved"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_slash_24() {
        let report = parse("10.0.0.0/24").expect("cidr");
        assert_eq!(report.family, Family::V4);
        assert_eq!(report.cidr(), "10.0.0.0/24");
        assert_eq!(report.prefix, 24);
        assert_eq!(report.netmask.as_deref(), Some("255.255.255.0"));
        assert_eq!(report.broadcast.as_deref(), Some("10.0.0.255"));
        assert_eq!(report.first, "10.0.0.1");
        assert_eq!(report.last, "10.0.0.254");
        assert_eq!(report.total, "256");
        assert_eq!(report.usable, "254");
        assert_eq!(report.scope, "private");
        assert_eq!(report.classification, "private (RFC 1918)");
        assert!(!report.normalized);
    }

    #[test]
    fn ipv4_edge_prefixes() {
        let point_to_point = parse("192.168.1.10/31").expect("/31");
        assert_eq!(point_to_point.network, "192.168.1.10");
        assert_eq!(point_to_point.broadcast.as_deref(), Some("192.168.1.11"));
        assert_eq!(point_to_point.first, "192.168.1.10");
        assert_eq!(point_to_point.last, "192.168.1.11");
        assert_eq!(point_to_point.total, "2");
        assert_eq!(point_to_point.usable, "2");

        let host = parse("8.8.8.8/32").expect("/32");
        assert_eq!(host.network, "8.8.8.8");
        assert_eq!(host.broadcast.as_deref(), Some("8.8.8.8"));
        assert_eq!(host.total, "1");
        assert_eq!(host.usable, "1");
        assert_eq!(host.scope, "global");
        assert_eq!(host.classification, "global (public)");

        let everything = parse("0.0.0.0/0").expect("/0");
        assert_eq!(everything.network, "0.0.0.0");
        assert_eq!(everything.broadcast.as_deref(), Some("255.255.255.255"));
        assert_eq!(everything.first, "0.0.0.1");
        assert_eq!(everything.last, "255.255.255.254");
        assert_eq!(everything.total, "4,294,967,296");
        assert_eq!(everything.usable, "4,294,967,294");
    }

    #[test]
    fn ipv4_normalises_host_bits() {
        let report = parse("10.0.0.5/24").expect("cidr");
        assert_eq!(report.input, "10.0.0.5/24");
        assert_eq!(report.network, "10.0.0.0");
        assert!(report.normalized);
        assert!(report.lines().contains("cleared (network shown)"));

        let bare = parse("10.0.0.5").expect("host");
        assert_eq!(bare.cidr(), "10.0.0.5/32");
        assert_eq!(bare.total, "1");
    }

    #[test]
    fn ipv4_scopes_and_classifications() {
        assert_eq!(parse("127.0.0.1/8").expect("loopback").scope, "loopback");
        assert_eq!(
            parse("169.254.1.1/16").expect("link-local").scope,
            "link-local"
        );
        assert_eq!(parse("239.1.2.3/4").expect("multicast").scope, "multicast");
        assert_eq!(parse("11.0.0.0/8").expect("public").scope, "global");
        assert_eq!(
            parse("172.16.0.0/12").expect("rfc1918").classification,
            "private (RFC 1918)"
        );
        assert_eq!(
            parse("100.64.0.0/10").expect("cgnat").classification,
            "shared — CGNAT (RFC 6598)"
        );
        assert_eq!(
            parse("192.0.2.0/24").expect("doc").classification,
            "documentation (RFC 5737)"
        );
        assert_eq!(
            parse("255.255.255.255/32")
                .expect("broadcast")
                .classification,
            "limited broadcast (RFC 919)"
        );
    }

    #[test]
    fn ipv6_slash_64() {
        let report = parse("2001:db8:1:2::/64").expect("v6 /64");
        assert_eq!(report.family, Family::V6);
        assert_eq!(report.cidr(), "2001:db8:1:2::/64");
        assert_eq!(report.first, "2001:db8:1:2::");
        assert_eq!(report.last, "2001:db8:1:2:ffff:ffff:ffff:ffff");
        assert_eq!(report.total, "18,446,744,073,709,551,616");
        assert_eq!(report.usable, report.total);
        assert!(report.netmask.is_none());
        assert!(report.broadcast.is_none());
        assert_eq!(report.scope, "global");
        assert_eq!(report.classification, "documentation (RFC 3849)");
    }

    #[test]
    fn ipv6_edge_prefixes() {
        let loopback = parse("::1/128").expect("loopback");
        assert_eq!(loopback.total, "1");
        assert_eq!(loopback.first, "::1");
        assert_eq!(loopback.last, "::1");
        assert_eq!(loopback.scope, "loopback");
        assert_eq!(loopback.classification, "loopback (RFC 4291)");

        let private = parse("fd00::/8").expect("ula");
        assert_eq!(private.scope, "private");
        assert_eq!(private.classification, "unique-local (RFC 4193)");

        let link_local = parse("fe80::/10").expect("link-local");
        assert_eq!(link_local.scope, "link-local");

        let multicast = parse("ff02::1/128").expect("multicast");
        assert_eq!(multicast.scope, "multicast");

        let everything = parse("::/0").expect("v6 /0");
        assert_eq!(everything.network, "::");
        assert_eq!(everything.last, "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff");
        assert_eq!(
            everything.total,
            "340,282,366,920,938,463,463,374,607,431,768,211,456"
        );
        assert_eq!(everything.usable, everything.total);
    }

    #[test]
    fn normalises_ipv6_host_bits() {
        let report = parse("2001:db8::dead:beef/64").expect("host bits");
        assert_eq!(report.cidr(), "2001:db8::/64");
        assert!(report.normalized);
    }

    #[test]
    fn rejects_invalid_cidrs() {
        for bad in [
            "",
            "10.0.0.0/33",
            "10.0.0.0/-1",
            "300.0.0.0/24",
            "2001:db8::/129",
            "10.0.0.0/abc",
            "not a cidr",
            "10.0.0.0/24/24",
            "/24",
        ] {
            assert!(parse(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(group_digits("0"), "0");
        assert_eq!(group_digits("999"), "999");
        assert_eq!(group_digits("1000"), "1,000");
        assert_eq!(group_digits("4294967296"), "4,294,967,296");
        assert_eq!(
            group_digits("340282366920938463463374607431768211456"),
            "340,282,366,920,938,463,463,374,607,431,768,211,456"
        );
    }
}
