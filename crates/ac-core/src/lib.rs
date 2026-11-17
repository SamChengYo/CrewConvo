//! Domain types, ids, errors and protocol definitions. No I/O.

pub mod bus;
pub mod error;
pub mod limits;
pub mod mention;
pub mod model;
pub mod protocol;
pub mod tool;

pub use error::{AppError, AppResult};

use uuid::Uuid;

pub type WorkspaceId = Uuid;
pub type MemberId = Uuid;
pub type AgentId = Uuid;
pub type ConversationId = Uuid;
pub type MessageId = Uuid;
pub type WakeId = Uuid;

/// All ids are UUID v7 (time ordered).
pub fn new_id() -> Uuid {
    Uuid::now_v7()
}

/// Generates a value for string-backed enums: `as_str`, `FromStr`, `Display`, serde as snake_case.
#[macro_export]
macro_rules! str_enum {
    ($(#[$m:meta])* $vis:vis enum $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        $vis enum $name {
            $(#[serde(rename = $s)] $variant),+
        }
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $s),+ }
            }
        }
        impl std::str::FromStr for $name {
            type Err = $crate::AppError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($s => Ok(Self::$variant),)+
                    other => Err($crate::AppError::Internal(format!(
                        concat!("invalid ", stringify!($name), ": {}"), other))),
                }
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// Rough token estimate used for payload budgets: CJK characters count as one token,
/// other characters as a quarter token.
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for c in text.chars() {
        if (c as u32) >= 0x2E80 {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + other.div_ceil(4)
}
