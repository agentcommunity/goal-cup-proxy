//! The provenance primitives a v5 chain carries: the lowercase SHA-256 digest, the canonical
//! UTC timestamp, the run-key spelling, and the termination outcome a local finish records
//! (D54).
//!
//! These types validate and bind declarations. They do not inspect a repository or read
//! captured objects.

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvenanceError {
    #[error("invalid lowercase SHA-256 digest")]
    InvalidSha256,
    #[error("invalid canonical UTC timestamp")]
    InvalidTimestamp,
    #[error("invalid {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("public key is not a usable Ed25519 key")]
    PublicKeyMalformed,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(hex::encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Sha256Digest {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_lower_hex(value, 64) {
            Ok(Self(value.to_string()))
        } else {
            Err(ProvenanceError::InvalidSha256)
        }
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Rfc3339Utc(String);

impl Rfc3339Utc {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn chronological_cmp(&self, other: &Self) -> Ordering {
        let whole = self.0[..19].cmp(&other.0[..19]);
        if whole != Ordering::Equal {
            return whole;
        }
        fractional_digits(&self.0).cmp(&fractional_digits(&other.0))
    }
}

impl TryFrom<&str> for Rfc3339Utc {
    type Error = ProvenanceError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_rfc3339_utc(value)?;
        Ok(Self(value.to_string()))
    }
}

impl TryFrom<String> for Rfc3339Utc {
    type Error = ProvenanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for Rfc3339Utc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// Return the canonical lowercase spelling of a usable, non-weak Ed25519 run key.
pub fn canonical_run_public_key(value: &str) -> Result<String, ProvenanceError> {
    if value.len() != 64 {
        return Err(ProvenanceError::PublicKeyMalformed);
    }
    let bytes: [u8; 32] = hex::decode(value)
        .map_err(|_| ProvenanceError::PublicKeyMalformed)?
        .try_into()
        .map_err(|_| ProvenanceError::PublicKeyMalformed)?;
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| ProvenanceError::PublicKeyMalformed)?;
    if key.is_weak() {
        return Err(ProvenanceError::PublicKeyMalformed);
    }
    Ok(hex::encode(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationMethodV1 {
    GracefulRequest,
    ProcessSignal,
    JobObjectClose,
    AlreadyExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationFailureV1 {
    ProcessAuthorityUnavailable,
    RequestFailed,
    ConfirmationTimedOut,
    ProcessStillRunning,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminationOutcomeV1 {
    AlreadyStopped {
        checked_at: Rfc3339Utc,
    },
    Confirmed {
        requested_at: Rfc3339Utc,
        confirmed_at: Rfc3339Utc,
        method: TerminationMethodV1,
    },
    Unresolved {
        requested_at: Rfc3339Utc,
        stopped_waiting_at: Rfc3339Utc,
        reason: TerminationFailureV1,
    },
}

impl TerminationOutcomeV1 {
    /// Check the outcome's own time ordering: a stop is not confirmed, and a wait does not
    /// end, before it was requested. The v5 local terminal writer and the phase replay apply
    /// this one rule.
    pub(crate) fn validate(&self) -> Result<(), ProvenanceError> {
        match self {
            Self::AlreadyStopped { .. } => Ok(()),
            Self::Confirmed {
                requested_at,
                confirmed_at,
                ..
            } => require_time_order(requested_at, confirmed_at, "termination"),
            Self::Unresolved {
                requested_at,
                stopped_waiting_at,
                ..
            } => require_time_order(requested_at, stopped_waiting_at, "termination wait"),
        }
    }
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_rfc3339_utc(value: &str) -> Result<(), ProvenanceError> {
    if value.len() < 20
        || value.len() > 30
        || !value.ends_with('Z')
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || value.as_bytes().get(10) != Some(&b'T')
        || value.as_bytes().get(13) != Some(&b':')
        || value.as_bytes().get(16) != Some(&b':')
    {
        return Err(ProvenanceError::InvalidTimestamp);
    }
    let digit = |range: std::ops::Range<usize>| -> Result<u32, ProvenanceError> {
        let bytes = value
            .as_bytes()
            .get(range)
            .ok_or(ProvenanceError::InvalidTimestamp)?;
        if !bytes.iter().all(u8::is_ascii_digit) {
            return Err(ProvenanceError::InvalidTimestamp);
        }
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or(ProvenanceError::InvalidTimestamp)
    };
    let year = digit(0..4)?;
    let month = digit(5..7)?;
    let day = digit(8..10)?;
    let hour = digit(11..13)?;
    let minute = digit(14..16)?;
    let second = digit(17..19)?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(ProvenanceError::InvalidTimestamp),
    };
    if year == 0 || day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return Err(ProvenanceError::InvalidTimestamp);
    }
    match value.as_bytes().get(19..value.len() - 1) {
        Some([]) => {}
        Some(fraction)
            if fraction.first() == Some(&b'.')
                && (1..=9).contains(&(fraction.len() - 1))
                && fraction[1..].iter().all(u8::is_ascii_digit) => {}
        _ => return Err(ProvenanceError::InvalidTimestamp),
    }
    Ok(())
}

fn fractional_digits(value: &str) -> [u8; 9] {
    let mut result = [b'0'; 9];
    if value.as_bytes().get(19) == Some(&b'.') {
        let digits = &value.as_bytes()[20..value.len() - 1];
        result[..digits.len()].copy_from_slice(digits);
    }
    result
}

pub(crate) fn require_time_order(
    first: &Rfc3339Utc,
    second: &Rfc3339Utc,
    field: &'static str,
) -> Result<(), ProvenanceError> {
    if first.chronological_cmp(second) == Ordering::Greater {
        Err(invalid(field, "timestamps are reversed"))
    } else {
        Ok(())
    }
}

fn invalid(field: &'static str, reason: &'static str) -> ProvenanceError {
    ProvenanceError::InvalidField { field, reason }
}
