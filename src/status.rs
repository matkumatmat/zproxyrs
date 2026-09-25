//! Printer status via Zebra `~HS` (Host Status) back-channel.
//! Port 9100 is fire-and-forget for print bytes; `~HS` is the read-back that
//! tells us paper/ribbon/head/pause state. Docs: zebra.com `~HS` + SGD
//! `device.host_status` (3 comma-separated strings, STX/ETX wrapped).
//! Rule #1: factory + generic fn + trait. Rule #2: `match` everywhere.

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::enums::PrintOutcome;

/// One hardware fault parsed from `~HS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrinterFault {
    PaperOut,
    Paused,
    HeadUp,
    RibbonOut,
    BufferFull,
    OverTemp,
    UnderTemp,
    CorruptRam,
    LabelWaiting,
    /// Printer stayed silent (per Zebra docs ~HS is suppressed on
    /// RIBBON OUT / HEAD OVER-TEMP / REWINDER FULL / critical faults).
    NoResponse,
}

impl PrinterFault {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PaperOut => "paper_out",
            Self::Paused => "paused",
            Self::HeadUp => "head_up",
            Self::RibbonOut => "ribbon_out",
            Self::BufferFull => "buffer_full",
            Self::OverTemp => "over_temp",
            Self::UnderTemp => "under_temp",
            Self::CorruptRam => "corrupt_ram",
            Self::LabelWaiting => "label_waiting",
            Self::NoResponse => "no_response",
        }
    }
}

/// Parsed `~HS` status. `raw` keeps the 3-line reply for debugging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PrinterStatus {
    pub paper_out: bool,
    pub paused: bool,
    pub buffer_full: bool,
    pub corrupt_ram: bool,
    pub under_temp: bool,
    pub over_temp: bool,
    pub head_up: bool,
    pub ribbon_out: bool,
    pub label_waiting: bool,
    pub formats_in_buffer: u32,
    pub labels_remaining: u32,
    pub raw: String,
}

impl Default for PrinterStatus {
    fn default() -> Self {
        Self::ready()
    }
}

impl PrinterStatus {
    /// Ideal state: everything clear.
    pub fn ready() -> Self {
        Self {
            paper_out: false,
            paused: false,
            buffer_full: false,
            corrupt_ram: false,
            under_temp: false,
            over_temp: false,
            head_up: false,
            ribbon_out: false,
            label_waiting: false,
            formats_in_buffer: 0,
            labels_remaining: 0,
            raw: String::new(),
        }
    }

    /// All active faults via pattern matching.
    pub fn faults(&self) -> Vec<PrinterFault> {
        let mut out = Vec::new();
        match self.paper_out {
            true => out.push(PrinterFault::PaperOut),
            false => {}
        }
        match self.paused {
            true => out.push(PrinterFault::Paused),
            false => {}
        }
        match self.head_up {
            true => out.push(PrinterFault::HeadUp),
            false => {}
        }
        match self.ribbon_out {
            true => out.push(PrinterFault::RibbonOut),
            false => {}
        }
        match self.buffer_full {
            true => out.push(PrinterFault::BufferFull),
            false => {}
        }
        match self.over_temp {
            true => out.push(PrinterFault::OverTemp),
            false => {}
        }
        match self.under_temp {
            true => out.push(PrinterFault::UnderTemp),
            false => {}
        }
        match self.corrupt_ram {
            true => out.push(PrinterFault::CorruptRam),
            false => {}
        }
        match self.label_waiting {
            true => out.push(PrinterFault::LabelWaiting),
            false => {}
        }
        out
    }

    /// Hard faults that mean "do not trust the print".
    pub fn is_ready(&self) -> bool {
        match (
            self.paper_out,
            self.paused,
            self.head_up,
            self.ribbon_out,
            self.over_temp,
            self.under_temp,
            self.corrupt_ram,
        ) {
            (false, false, false, false, false, false, false) => true,
            _ => false,
        }
    }

    /// Soft warnings: sent but operator should look (buffer/label waiting).
    pub fn has_warnings(&self) -> bool {
        match (self.buffer_full, self.label_waiting) {
            (true, _) => true,
            (_, true) => true,
            _ => false,
        }
    }

    /// Status -> bridge outcome via pattern matching (single mapping point).
    pub fn outcome(&self) -> PrintOutcome {
        match (self.is_ready(), self.has_warnings()) {
            (true, false) => PrintOutcome::Sent,
            (true, true) => PrintOutcome::SentWithWarnings,
            (false, _) => PrintOutcome::Rejected,
        }
    }
}

/// Parse failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusParseError {
    Empty,
    MissingLine { got: usize },
    BadField { line: usize, field: String },
}

impl std::fmt::Display for StatusParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty status reply"),
            Self::MissingLine { got } => write!(f, "expected 3 status lines, got {got}"),
            Self::BadField { line, field } => write!(f, "line {line}: bad field {field:?}"),
        }
    }
}

impl std::error::Error for StatusParseError {}

/// Factory for status values (Rule #1).
pub struct StatusFactory;

impl StatusFactory {
    /// Generic parse over any string buffer (Rule #1: generic fn).
    pub fn parse<T: AsRef<str>>(raw: &T) -> Result<PrinterStatus, StatusParseError> {
        parse_status(raw.as_ref())
    }

    pub fn ready() -> PrinterStatus {
        PrinterStatus::ready()
    }

    pub fn no_response() -> PrinterStatus {
        PrinterStatus {
            raw: String::new(),
            ..PrinterStatus::ready()
        }
    }
}

/// Parse `~HS` reply: 3 lines, STX/ETX/CR/LF tolerant.
///
/// Line1: `aaa,b,c,dddd,eee,f,g,h,iii,j,k,l` (12 fields)
/// Line2: `mmm,n,o,p,q,r,s,t,uuuuuuuu,v,www` (11 fields)
/// Line3: `xxxx,y` (ignored except kept in raw)
pub fn parse_status(raw: &str) -> Result<PrinterStatus, StatusParseError> {
    // Strip control chars, drop empties, pattern match line count.
    let cleaned: String = raw
        .chars()
        .filter(|c| *c != '\x02' && *c != '\x03')
        .collect();
    let lines: Vec<&str> = cleaned
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    let (l1, l2, _l3) = match lines.as_slice() {
        [a, b, c, ..] => (*a, *b, *c),
        _ => {
            return Err(StatusParseError::MissingLine { got: lines.len() });
        }
    };

    let f1: Vec<&str> = l1.split(',').map(str::trim).collect();
    let f2: Vec<&str> = l2.split(',').map(str::trim).collect();

    match (f1.len(), f2.len()) {
        (12, 11) => {}
        _ => {
            return Err(StatusParseError::MissingLine { got: lines.len() });
        }
    }

    // Generic flag parser: "1" => true, "0" => false, else error.
    let flag = |line: usize, v: &str| -> Result<bool, StatusParseError> {
        match v {
            "1" => Ok(true),
            "0" => Ok(false),
            // Zebra pads some flags; treat any nonzero digit run as true.
            _ => match v.trim().parse::<u32>() {
                Ok(0) => Ok(false),
                Ok(_) => Ok(true),
                Err(_) => Err(StatusParseError::BadField {
                    line,
                    field: v.to_string(),
                }),
            },
        }
    };
    let count = |line: usize, v: &str| -> Result<u32, StatusParseError> {
        match v.trim().parse::<u32>() {
            Ok(n) => Ok(n),
            Err(_) => Err(StatusParseError::BadField {
                line,
                field: v.to_string(),
            }),
        }
    };

    Ok(PrinterStatus {
        paper_out: flag(1, f1[1])?,
        paused: flag(1, f1[2])?,
        formats_in_buffer: count(1, f1[4])?,
        buffer_full: flag(1, f1[5])?,
        corrupt_ram: flag(1, f1[9])?,
        under_temp: flag(1, f1[10])?,
        over_temp: flag(1, f1[11])?,
        head_up: flag(2, f2[2])?,
        ribbon_out: flag(2, f2[3])?,
        label_waiting: flag(2, f2[6])?,
        labels_remaining: count(2, f2[7])?,
        raw: raw.to_string(),
    })
}

/// Trait for anything that can probe printer health (Rule #1: trait).
pub trait StatusProber {
    fn query_status<'a>(
        &'a self,
    ) -> impl std::future::Future<Output = Result<PrinterStatus, StatusProbeError>> + Send + 'a;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusProbeError {
    Connect(String),
    Write(String),
    Read(String),
    Timeout,
    Parse(StatusParseError),
}

impl std::fmt::Display for StatusProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(a) => write!(f, "status connect failed: {a}"),
            Self::Write(a) => write!(f, "status write failed: {a}"),
            Self::Read(a) => write!(f, "status read failed: {a}"),
            Self::Timeout => write!(
                f,
                "status probe timeout (printer may hold ~HS on critical fault)"
            ),
            Self::Parse(e) => write!(f, "status parse failed: {e}"),
        }
    }
}

impl std::error::Error for StatusProbeError {}

/// Keep numeric knobs in CONST (satisfies Rule #3 alongside enums).
pub fn status_timeouts() -> (u64, u64) {
    match (TCP_CONNECT_TIMEOUT_MS, STATUS_READ_TIMEOUT_MS) {
        (c, r) => (c, r),
    }
}
