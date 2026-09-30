//! Explicit write dates and content-derived PDF identity. No ambient clock is
//! consulted unless the caller explicitly selects `current`.

use fullbleed_audit_contract::sha256::Sha256;
use std::{
    fmt, io,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfTimestamp(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfTimestampError(&'static str);

impl fmt::Display for PdfTimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PDF_TIMESTAMP_INVALID: {}", self.0)
    }
}
impl std::error::Error for PdfTimestampError {}

impl PdfTimestamp {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Resolve the clock once for a job; reuse this value for reproducible replay.
    pub fn current() -> Result<Self, PdfTimestampError> {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PdfTimestampError("clock precedes the Unix epoch"))?
            .as_secs();
        Self::from_unix_seconds(seconds)
    }

    pub fn from_source_date_epoch() -> Result<Self, PdfTimestampError> {
        let value = std::env::var("SOURCE_DATE_EPOCH")
            .map_err(|_| PdfTimestampError("SOURCE_DATE_EPOCH is not set"))?;
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PdfTimestampError(
                "SOURCE_DATE_EPOCH must be nonnegative integer seconds",
            ));
        }
        Self::from_unix_seconds(
            value
                .parse()
                .map_err(|_| PdfTimestampError("SOURCE_DATE_EPOCH is out of range"))?,
        )
    }

    pub fn from_unix_seconds(seconds: u64) -> Result<Self, PdfTimestampError> {
        if seconds > 253_402_300_799 {
            return Err(PdfTimestampError(
                "timestamp must be within years 1970 through 9999",
            ));
        }
        let mut days = seconds / 86_400;
        let mut year = 1970;
        while days >= days_in_year(year) {
            days -= days_in_year(year);
            year += 1;
        }
        let mut month = 1;
        while days >= days_in_month(year, month) {
            days -= days_in_month(year, month);
            month += 1;
        }
        Ok(Self(format!(
            "{year:04}-{month:02}-{:02}T{:02}:{:02}:{:02}Z",
            days + 1,
            seconds / 3600 % 24,
            seconds / 60 % 60,
            seconds % 60
        )))
    }

    pub(crate) fn pdf_date(&self) -> String {
        format!(
            "D:{}Z",
            self.0
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
        )
    }
}

fn days_in_year(year: u64) -> u64 {
    if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
        366
    } else {
        365
    }
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 => {
            if days_in_year(year) == 366 {
                29
            } else {
                28
            }
        }
        _ => 31,
    }
}

impl FromStr for PdfTimestamp {
    type Err = PdfTimestampError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "current" => return Self::current(),
            "source-date-epoch" => return Self::from_source_date_epoch(),
            _ => {}
        }
        let invalid =
            PdfTimestampError("expected current, source-date-epoch, or YYYY-MM-DDTHH:MM:SSZ (UTC)");
        let bytes = value.as_bytes();
        if bytes.len() != 20
            || !bytes.iter().enumerate().all(|(i, b)| match i {
                4 | 7 => *b == b'-',
                10 => *b == b'T',
                13 | 16 => *b == b':',
                19 => *b == b'Z',
                _ => b.is_ascii_digit(),
            })
        {
            return Err(invalid);
        }
        let number = |start, end| value[start..end].parse::<u64>().unwrap();
        let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
        if year == 0
            || !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || number(11, 13) > 23
            || number(14, 16) > 59
            || number(17, 19) > 59
        {
            return Err(PdfTimestampError(
                "invalid Gregorian calendar date or UTC time",
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

pub(crate) struct IdentityWriter<'a, W: io::Write> {
    inner: &'a mut W,
    hash: Option<Sha256>,
}

impl<'a, W: io::Write> IdentityWriter<'a, W> {
    pub(crate) fn new(inner: &'a mut W, enabled: bool) -> Self {
        Self {
            inner,
            hash: enabled.then(Sha256::new),
        }
    }

    pub(crate) fn identity(
        &mut self,
        timestamp: PdfTimestamp,
        fields: &[&[u8]],
    ) -> Option<PdfIdentity> {
        let mut hash = self.hash.take()?;
        for field in fields {
            hash.update(&(field.len() as u64).to_be_bytes());
            hash.update(field);
        }
        // The content identity excludes the write date. A new write of the same
        // document retains DocumentID but gets a distinct InstanceID/trailer ID.
        let document = hash.clone().finalize();
        hash.update(timestamp.as_str().as_bytes());
        hash.update(b"fullbleed-pdf-instance-v1");
        let instance = hash.finalize();
        Some(PdfIdentity {
            timestamp,
            document_id: uuid(&document),
            instance_id: uuid(&instance),
            file_id: instance[..16].iter().map(|b| format!("{b:02X}")).collect(),
        })
    }
}

impl<W: io::Write> io::Write for IdentityWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(bytes)?;
        if let Some(hash) = &mut self.hash {
            hash.update(&bytes[..count]);
        }
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn uuid(hash: &[u8; 32]) -> String {
    let mut bytes: [u8; 16] = hash[..16].try_into().unwrap();
    bytes[6] = (bytes[6] & 15) | 0x80; // RFC 9562 UUIDv8, application-defined SHA-256.
    bytes[8] = (bytes[8] & 63) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

pub(crate) struct PdfIdentity {
    pub timestamp: PdfTimestamp,
    pub document_id: String,
    pub instance_id: String,
    pub file_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamp_calendar_boundaries() {
        for (seconds, expected) in [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (253_402_300_799, "9999-12-31T23:59:59Z"),
        ] {
            assert_eq!(
                PdfTimestamp::from_unix_seconds(seconds).unwrap().as_str(),
                expected
            );
            assert_eq!(expected.parse::<PdfTimestamp>().unwrap().as_str(), expected);
        }
        assert!(PdfTimestamp::from_unix_seconds(u64::MAX).is_err());
        for bad in [
            "2026-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "2026-09-31T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:00:60Z",
            "0000-01-01T00:00:00Z",
            "2026-01-01",
            "2026-01-01T00:00:00+00:00",
            "é000-01-01T00:00:00Z",
        ] {
            assert!(bad.parse::<PdfTimestamp>().is_err(), "accepted {bad}");
        }
        assert_eq!(
            "2026-09-30T12:34:56Z"
                .parse::<PdfTimestamp>()
                .unwrap()
                .pdf_date(),
            "D:20260930123456Z"
        );
    }
}
