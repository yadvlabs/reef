//! Time-ordered 64-bit identifiers generated in the application, so no database
//! sequence is needed and any number of Reef hosts can allocate IDs concurrently.
//!
//! Layout (53 bits, so the value survives JavaScript's `Number` and JSON parsers that use
//! doubles): 41 bits of milliseconds since the Reef epoch (69 years), 4 bits of node id
//! (16 hosts), 8 bits of per-millisecond sequence (256 ids per ms per host; the generator
//! borrows the next millisecond when exhausted).

use std::sync::Mutex;
use std::sync::OnceLock;

/// Milliseconds between the Unix epoch and 2026-01-01T00:00:00Z.
pub const REEF_EPOCH_MS: i64 = 1_767_225_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Id(pub i64);

impl Id {
    pub fn as_i64(self) -> i64 {
        self.0
    }
    /// Milliseconds since the Unix epoch at which this id was minted.
    pub fn created_ms(self) -> i64 {
        (self.0 >> 12) + REEF_EPOCH_MS
    }

    /// Largest id that JavaScript represents exactly.
    pub const MAX: i64 = (1i64 << 53) - 1;
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for Id {
    fn from(v: i64) -> Self {
        Id(v)
    }
}

impl std::str::FromStr for Id {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<i64>().map(Id)
    }
}

pub struct IdGen {
    node: i64,
    state: Mutex<(i64, i64)>, // (last_ms, seq)
}

impl IdGen {
    pub fn new(node: u16) -> Self {
        IdGen { node: (node & 0xf) as i64, state: Mutex::new((0, 0)) }
    }

    pub fn next(&self) -> Id {
        let mut st = self.state.lock().unwrap();
        let mut now = crate::time::now_ms() - REEF_EPOCH_MS;
        if now < st.0 {
            now = st.0; // clock went backwards; keep monotonic
        }
        if now == st.0 {
            st.1 = (st.1 + 1) & 0xff;
            if st.1 == 0 {
                // sequence exhausted this millisecond; borrow the next one
                now += 1;
            }
        } else {
            st.1 = 0;
        }
        st.0 = now;
        Id((now << 12) | (self.node << 8) | st.1)
    }
}

static GLOBAL: OnceLock<IdGen> = OnceLock::new();

/// Install the process-wide generator. Called once at startup with the node id from config.
pub fn install(node: u16) {
    let _ = GLOBAL.set(IdGen::new(node));
}

/// Mint a new id from the process-wide generator (node 0 if `install` was never called).
pub fn next() -> Id {
    GLOBAL.get_or_init(|| IdGen::new(0)).next()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_monotonic_and_unique() {
        let g = IdGen::new(7);
        let mut last = g.next();
        for _ in 0..10_000 {
            let n = g.next();
            assert!(n > last);
            last = n;
        }
        assert_eq!((last.0 >> 8) & 0xf, 7);
        assert!(last.0 < Id::MAX);
        assert!(last.created_ms() > REEF_EPOCH_MS);
    }
}
