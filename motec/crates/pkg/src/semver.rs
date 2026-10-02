use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// A SemVer 2.0 version.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Pre-release identifiers; empty for a release.
    pub pre: Vec<String>,
    /// Build metadata; empty when absent.
    pub build: String,
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Version::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl Version {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self { major, minor, patch, pre: Vec::new(), build: String::new() }
    }

    pub(crate) fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }

    fn same_release(&self, other: &Version) -> bool {
        (self.major, self.minor, self.patch) == (other.major, other.minor, other.patch)
    }

    fn precedence(&self, other: &Self) -> Ordering {
        let core = (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
        core.then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let ord = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Ordering::Less,
                        (Err(_), Ok(_)) => Ordering::Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.precedence(other).then_with(|| self.build.cmp(&other.build))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        if !self.build.is_empty() {
            write!(f, "+{}", self.build)?;
        }
        Ok(())
    }
}

impl FromStr for Version {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let bad = |why: &str| format!("invalid version '{s}': {why}");
        let (rest, build) = match s.split_once('+') {
            Some((r, b)) => (r, Some(b)),
            None => (s, None),
        };
        let (core, pre) = match rest.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (rest, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        let full = pre.is_some() || build.is_some();
        if parts.len() > 3 || parts.len() < 2 || (full && parts.len() != 3) {
            return Err(bad("expected major.minor.patch"));
        }
        let num = |p: &str| -> Result<u64, String> {
            if p.len() > 1 && p.starts_with('0') {
                return Err(bad("a number has a leading zero"));
            }
            p.parse::<u64>().map_err(|_| bad(&format!("'{p}' is not a number")))
        };
        let ident_ok = |id: &str| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let mut v = Version::new(num(parts[0])?, num(parts[1])?, if parts.len() == 3 { num(parts[2])? } else { 0 });
        if let Some(pre) = pre {
            for id in pre.split('.') {
                if !ident_ok(id) {
                    return Err(bad("pre-release identifiers are non-empty [0-9A-Za-z-]"));
                }
                if id.len() > 1 && id.starts_with('0') && id.chars().all(|c| c.is_ascii_digit()) {
                    return Err(bad("a numeric pre-release identifier has a leading zero"));
                }
                v.pre.push(id.to_string());
            }
        }
        if let Some(build) = build {
            if !build.split('.').all(ident_ok) {
                return Err(bad("build identifiers are non-empty [0-9A-Za-z-]"));
            }
            v.build = build.to_string();
        }
        Ok(v)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A version comparison operator.
pub enum VersionOp {
    Exact,
    Caret,
    Tilde,
    Greater,
    GreaterEq,
    Less,
    LessEq,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// One operator and version, such as `>=1.2.0`.
pub struct VersionConstraint {
    pub op: VersionOp,
    pub version: Version,
}

impl VersionConstraint {
    /// Compares by precedence, so build metadata never matters.
    pub fn matches(&self, ver: &Version) -> bool {
        let v = &self.version;
        let ord = ver.precedence(v);
        match self.op {
            VersionOp::Exact => ord == Ordering::Equal,
            VersionOp::Greater => ord == Ordering::Greater,
            VersionOp::GreaterEq => ord != Ordering::Less,
            VersionOp::Less => ord == Ordering::Less,
            VersionOp::LessEq => ord != Ordering::Greater,
            VersionOp::Tilde => ver.major == v.major && ver.minor == v.minor && ord != Ordering::Less,
            VersionOp::Caret => {
                ord != Ordering::Less
                    && if v.major > 0 {
                        ver.major == v.major
                    } else if v.minor > 0 {
                        ver.major == 0 && ver.minor == v.minor
                    } else {
                        ver.same_release(v)
                    }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// A comma-separated list of constraints.
pub struct VersionReq {
    pub raw: String,
    pub constraints: Vec<VersionConstraint>,
}

impl VersionReq {
    pub fn parse(s: &str) -> Result<Self, String> {
        let trimmed = s.trim();
        if trimmed == "*" || trimmed.is_empty() {
            return Ok(Self {
                raw: trimmed.to_string(),
                constraints: vec![VersionConstraint {
                    op: VersionOp::GreaterEq,
                    version: Version::new(0, 0, 0),
                }],
            });
        }

        let parts: Vec<&str> = trimmed.split(',').collect();
        let mut constraints = Vec::new();

        for part in parts {
            let p = part.trim();
            let prefixes = [
                (">=", VersionOp::GreaterEq),
                ("<=", VersionOp::LessEq),
                (">", VersionOp::Greater),
                ("<", VersionOp::Less),
                ("^", VersionOp::Caret),
                ("~", VersionOp::Tilde),
                ("=", VersionOp::Exact),
            ];
            let (op, ver_str) = prefixes
                .into_iter()
                .find_map(|(prefix, op)| p.strip_prefix(prefix).map(|rest| (op, rest)))
                .unwrap_or((VersionOp::Caret, p));

            let ver = ver_str.trim().parse::<Version>()?;
            constraints.push(VersionConstraint { op, version: ver });
        }

        Ok(Self {
            raw: trimmed.to_string(),
            constraints,
        })
    }

    /// A pre-release matches only when a comparator names a pre-release of its `major.minor.patch`.
    pub fn matches(&self, ver: &Version) -> bool {
        self.constraints.iter().all(|c| c.matches(ver))
            && (!ver.is_prerelease()
                || self.constraints.iter().any(|c| c.version.is_prerelease() && c.version.same_release(ver)))
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn req(s: &str) -> VersionReq {
        VersionReq::parse(s).unwrap()
    }

    #[test]
    fn parses_and_prints_prereleases_and_build_metadata() {
        let x = v("1.0.0-rc.1+build.5");
        assert_eq!((x.pre.clone(), x.build.as_str()), (vec!["rc".to_string(), "1".to_string()], "build.5"));
        assert_eq!(x.to_string(), "1.0.0-rc.1+build.5");
        assert_eq!(v("1.2"), Version::new(1, 2, 0));
        for bad in ["1.2-rc", "1.0.0-", "1.0.0-a..b", "1.0.0-01", "01.0.0", "1.0.0+", "1.0.0-a_b", "1.0.0.0"] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
    }

    #[test]
    fn orders_by_semver_precedence() {
        let order = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1"];
        for pair in order.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_ne!(v("1.0.0+a"), v("1.0.0+b"));
        assert!(req("=1.0.0").matches(&v("1.0.0+anything")));
    }

    #[test]
    fn prereleases_match_only_when_named() {
        assert!(!req("^1.0.0").matches(&v("1.1.0-beta")));
        assert!(!req("*").matches(&v("1.0.0-rc.1")));
        assert!(!req("<2.0.0").matches(&v("2.0.0-alpha")));
        let beta = req("^1.1.0-beta.2");
        assert!(beta.matches(&v("1.1.0-beta.3")) && beta.matches(&v("1.1.0")) && beta.matches(&v("1.4.0")));
        assert!(!beta.matches(&v("1.1.0-beta.1")) && !beta.matches(&v("1.2.0-alpha")) && !beta.matches(&v("2.0.0")));
        assert!(req("~0.3.0-rc.1").matches(&v("0.3.0-rc.2")) && !req("~0.3.0-rc.1").matches(&v("0.4.0")));
        assert!(req("^0.0.3-a").matches(&v("0.0.3")) && !req("^0.0.3-a").matches(&v("0.0.4")));
    }
}
