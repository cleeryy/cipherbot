//! CVSS 3.0 / 3.1 base score calculator for `/tool cvss`.
//!
//! Implements the official base formula (CVSS v3.1 specification, section 7.1)
//! including the `Roundup` helper. Temporal and environmental metrics are
//! accepted in the vector but ignored for the base score.

use std::collections::HashSet;
use std::fmt;

use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V30,
    V31,
}

impl Version {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V30 => "3.0",
            Self::V31 => "3.1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackVector {
    Network,
    Adjacent,
    Local,
    Physical,
}

impl AttackVector {
    pub fn code(self) -> &'static str {
        match self {
            Self::Network => "N",
            Self::Adjacent => "A",
            Self::Local => "L",
            Self::Physical => "P",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Network => "Network (N)",
            Self::Adjacent => "Adjacent (A)",
            Self::Local => "Local (L)",
            Self::Physical => "Physical (P)",
        }
    }

    fn weight(self) -> f64 {
        match self {
            Self::Network => 0.85,
            Self::Adjacent => 0.62,
            Self::Local => 0.55,
            Self::Physical => 0.2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackComplexity {
    Low,
    High,
}

impl AttackComplexity {
    pub fn code(self) -> &'static str {
        match self {
            Self::Low => "L",
            Self::High => "H",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low (L)",
            Self::High => "High (H)",
        }
    }

    fn weight(self) -> f64 {
        match self {
            Self::Low => 0.77,
            Self::High => 0.44,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegesRequired {
    None,
    Low,
    High,
}

impl PrivilegesRequired {
    pub fn code(self) -> &'static str {
        match self {
            Self::None => "N",
            Self::Low => "L",
            Self::High => "H",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None (N)",
            Self::Low => "Low (L)",
            Self::High => "High (H)",
        }
    }

    fn weight(self, scope: Scope) -> f64 {
        match (self, scope) {
            (Self::None, _) => 0.85,
            (Self::Low, Scope::Unchanged) => 0.62,
            (Self::Low, Scope::Changed) => 0.68,
            (Self::High, Scope::Unchanged) => 0.27,
            (Self::High, Scope::Changed) => 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserInteraction {
    None,
    Required,
}

impl UserInteraction {
    pub fn code(self) -> &'static str {
        match self {
            Self::None => "N",
            Self::Required => "R",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None (N)",
            Self::Required => "Required (R)",
        }
    }

    fn weight(self) -> f64 {
        match self {
            Self::None => 0.85,
            Self::Required => 0.62,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Unchanged,
    Changed,
}

impl Scope {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unchanged => "U",
            Self::Changed => "C",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "Unchanged (U)",
            Self::Changed => "Changed (C)",
        }
    }
}

/// Impact rating shared by the confidentiality, integrity and availability metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cia {
    High,
    Low,
    None,
}

impl Cia {
    pub fn code(self) -> &'static str {
        match self {
            Self::High => "H",
            Self::Low => "L",
            Self::None => "N",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::High => "High (H)",
            Self::Low => "Low (L)",
            Self::None => "None (N)",
        }
    }

    fn weight(self) -> f64 {
        match self {
            Self::High => 0.56,
            Self::Low => 0.22,
            Self::None => 0.0,
        }
    }
}

/// Parsed base metrics of a CVSS 3.0 / 3.1 vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CvssVector {
    pub version: Version,
    pub av: AttackVector,
    pub ac: AttackComplexity,
    pub pr: PrivilegesRequired,
    pub ui: UserInteraction,
    pub scope: Scope,
    pub confidentiality: Cia,
    pub integrity: Cia,
    pub availability: Cia,
}

/// Metrics that exist in CVSS but do not contribute to the base score.
const OPTIONAL_METRICS: &[&str] = &[
    "E", "RL", "RC", "CR", "IR", "AR", "MAV", "MAC", "MPR", "MUI", "MS", "MC", "MI", "MA",
];

impl fmt::Display for CvssVector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "CVSS:{}/AV:{}/AC:{}/PR:{}/UI:{}/S:{}/C:{}/I:{}/A:{}",
            self.version.as_str(),
            self.av.code(),
            self.ac.code(),
            self.pr.code(),
            self.ui.code(),
            self.scope.code(),
            self.confidentiality.code(),
            self.integrity.code(),
            self.availability.code(),
        )
    }
}

/// Parse a CVSS 3.0 / 3.1 vector, validating every required base metric.
pub fn parse_vector(vector: &str) -> Result<CvssVector> {
    let raw = vector.trim();
    if raw.is_empty() {
        bail!("Give a CVSS vector such as CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H.");
    }

    let mut parts = raw.split('/');
    let header = parts.next().ok_or_else(|| anyhow!("Empty CVSS vector."))?;
    let version = match header.trim().to_ascii_uppercase().as_str() {
        "CVSS:3.0" => Version::V30,
        "CVSS:3.1" => Version::V31,
        _ => bail!("Vector must start with `CVSS:3.0` or `CVSS:3.1`."),
    };

    let mut av = None;
    let mut ac = None;
    let mut pr = None;
    let mut ui = None;
    let mut scope = None;
    let mut confidentiality = None;
    let mut integrity = None;
    let mut availability = None;
    let mut seen: HashSet<String> = HashSet::new();

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            bail!("Empty metric in vector.");
        }
        let (key, value) = part
            .split_once(':')
            .ok_or_else(|| anyhow!("Metric `{part}` is missing a value."))?;
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim().to_ascii_uppercase();
        if value.is_empty() {
            bail!("Metric `{key}` is missing a value.");
        }
        if !seen.insert(key.clone()) {
            bail!("Duplicate metric `{key}` in vector.");
        }

        match key.as_str() {
            "AV" => {
                av = Some(match value.as_str() {
                    "N" => AttackVector::Network,
                    "A" => AttackVector::Adjacent,
                    "L" => AttackVector::Local,
                    "P" => AttackVector::Physical,
                    other => bail!("Invalid AV value `{other}` (expected N, A, L or P)."),
                });
            }
            "AC" => {
                ac = Some(match value.as_str() {
                    "L" => AttackComplexity::Low,
                    "H" => AttackComplexity::High,
                    other => bail!("Invalid AC value `{other}` (expected L or H)."),
                });
            }
            "PR" => {
                pr = Some(match value.as_str() {
                    "N" => PrivilegesRequired::None,
                    "L" => PrivilegesRequired::Low,
                    "H" => PrivilegesRequired::High,
                    other => bail!("Invalid PR value `{other}` (expected N, L or H)."),
                });
            }
            "UI" => {
                ui = Some(match value.as_str() {
                    "N" => UserInteraction::None,
                    "R" => UserInteraction::Required,
                    other => bail!("Invalid UI value `{other}` (expected N or R)."),
                });
            }
            "S" => {
                scope = Some(match value.as_str() {
                    "U" => Scope::Unchanged,
                    "C" => Scope::Changed,
                    other => bail!("Invalid S value `{other}` (expected U or C)."),
                });
            }
            "C" => {
                confidentiality = Some(parse_cia("C", &value)?);
            }
            "I" => {
                integrity = Some(parse_cia("I", &value)?);
            }
            "A" => {
                availability = Some(parse_cia("A", &value)?);
            }
            other if OPTIONAL_METRICS.contains(&other) => {}
            other => bail!("Unknown metric `{other}`."),
        }
    }

    Ok(CvssVector {
        version,
        av: av.ok_or_else(|| anyhow!("Missing required metric AV."))?,
        ac: ac.ok_or_else(|| anyhow!("Missing required metric AC."))?,
        pr: pr.ok_or_else(|| anyhow!("Missing required metric PR."))?,
        ui: ui.ok_or_else(|| anyhow!("Missing required metric UI."))?,
        scope: scope.ok_or_else(|| anyhow!("Missing required metric S."))?,
        confidentiality: confidentiality.ok_or_else(|| anyhow!("Missing required metric C."))?,
        integrity: integrity.ok_or_else(|| anyhow!("Missing required metric I."))?,
        availability: availability.ok_or_else(|| anyhow!("Missing required metric A."))?,
    })
}

fn parse_cia(metric: &str, value: &str) -> Result<Cia> {
    match value {
        "H" => Ok(Cia::High),
        "L" => Ok(Cia::Low),
        "N" => Ok(Cia::None),
        other => bail!("Invalid {metric} value `{other}` (expected H, L or N)."),
    }
}

/// Official CVSS base score, rounded up to one decimal.
pub fn base_score(vector: &CvssVector) -> f64 {
    let iss = 1.0
        - (1.0 - vector.confidentiality.weight())
            * (1.0 - vector.integrity.weight())
            * (1.0 - vector.availability.weight());

    let impact = match vector.scope {
        Scope::Unchanged => 6.42 * iss,
        Scope::Changed => 7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15),
    };
    if impact <= 0.0 {
        return 0.0;
    }

    let exploitability = 8.22
        * vector.av.weight()
        * vector.ac.weight()
        * vector.pr.weight(vector.scope)
        * vector.ui.weight();

    let raw = match vector.scope {
        Scope::Unchanged => impact + exploitability,
        Scope::Changed => 1.08 * (impact + exploitability),
    };

    roundup(raw.min(10.0))
}

/// CVSS v3.1 `Roundup`: smallest one-decimal value greater than or equal to the input.
fn roundup(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let scaled = (value * 100_000.0).round() as i64;
    if scaled.rem_euclid(10_000) == 0 {
        scaled as f64 / 100_000.0
    } else {
        ((scaled.div_euclid(10_000) + 1) as f64) / 10.0
    }
}

pub fn severity(score: f64) -> &'static str {
    if score >= 9.0 {
        "Critical"
    } else if score >= 7.0 {
        "High"
    } else if score >= 4.0 {
        "Medium"
    } else if score > 0.0 {
        "Low"
    } else {
        "None"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    fn score(vector: &str) -> f64 {
        base_score(&parse_vector(vector).expect("valid vector"))
    }

    #[test]
    fn critical_network_vectors() {
        // Official spec: ISS = 1 - 0.44^3 = 0.914816, so scope Unchanged caps at 9.8
        // (same score as BlueKeep, CVE-2019-0708).
        assert!(approx(
            score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            9.8
        ));
        assert!(approx(
            score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"),
            10.0
        ));
    }

    #[test]
    fn changed_scope_xss_vector_is_6_1() {
        let vector = parse_vector("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N").expect("parse");
        assert_eq!(vector.scope, Scope::Changed);
        assert!(approx(base_score(&vector), 6.1));
        assert_eq!(severity(6.1), "Medium");
    }

    #[test]
    fn local_high_complexity_vector_is_1_8() {
        assert!(approx(
            score("CVSS:3.1/AV:L/AC:H/PR:H/UI:R/S:U/C:L/I:N/A:N"),
            1.8
        ));
    }

    #[test]
    fn low_complexity_authenticated_vector_is_4_3() {
        assert!(approx(
            score("CVSS:3.0/AV:N/AC:L/PR:L/UI:N/S:U/C:L/I:N/A:N"),
            4.3
        ));
    }

    #[test]
    fn zero_impact_scores_zero() {
        assert!(approx(
            score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:N"),
            0.0
        ));
        assert_eq!(severity(0.0), "None");
    }

    #[test]
    fn optional_metrics_are_ignored() {
        let plain = score("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N");
        let decorated =
            score("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N/E:P/RL:O/RC:C/CR:H/IR:M/AR:L");
        assert!(approx(plain, decorated));
    }

    #[test]
    fn lowercase_vectors_parse() {
        let vector = parse_vector("cvss:3.1/av:n/ac:l/pr:n/ui:n/s:u/c:h/i:h/a:h").expect("parse");
        assert_eq!(
            vector.to_string(),
            "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
        );
    }

    #[test]
    fn display_roundtrips() {
        let raw = "CVSS:3.0/AV:A/AC:H/PR:L/UI:R/S:U/C:L/I:H/A:N";
        let vector = parse_vector(raw).expect("parse");
        assert_eq!(vector.to_string(), raw);
        assert_eq!(parse_vector(&vector.to_string()).expect("reparse"), vector);
    }

    #[test]
    fn rejects_malformed_vectors() {
        for bad in [
            "",
            "AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
            "CVSS:2.0/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
            "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/C:H/I:H/A:H",
            "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H/AV:N",
            "CVSS:3.1/AV:X/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
            "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H/XX:Y",
            "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H",
            "CVSS:3.1/AVN/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
        ] {
            assert!(parse_vector(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn severity_boundaries() {
        assert_eq!(severity(0.0), "None");
        assert_eq!(severity(0.1), "Low");
        assert_eq!(severity(3.9), "Low");
        assert_eq!(severity(4.0), "Medium");
        assert_eq!(severity(6.9), "Medium");
        assert_eq!(severity(7.0), "High");
        assert_eq!(severity(8.9), "High");
        assert_eq!(severity(9.0), "Critical");
        assert_eq!(severity(10.0), "Critical");
    }
}
