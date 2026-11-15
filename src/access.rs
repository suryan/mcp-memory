//! Access modes and the server-side check that enforces them.

use serde::{Deserialize, Serialize};

/// Who is calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// A coding agent calling MCP tools. Subject to the access mode.
    Agent,
    /// A person using the CLI. Always allowed, so delete works when the agent cannot write.
    Human,
}

/// What the caller wants to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Recall, list, show, or inspect.
    Read,
    /// A write the user asked for.
    ExplicitWrite,
    /// A write the agent chose to make.
    AutomaticWrite,
}

/// How much the agent may do. Set by the user in `config.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// No recall and no writes.
    Off,
    /// Recall and list only.
    ReadOnly,
    /// Recall, plus writes the user asked for.
    ReadWrite,
    /// Recall, explicit writes, and automatic writes.
    Learn,
}

impl Access {
    /// Parse a mode name. Accepts `read_only`, `readonly`, and `read-only` spellings.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "read_only" | "readonly" | "read-only" => Some(Self::ReadOnly),
            "read_write" | "readwrite" | "read-write" => Some(Self::ReadWrite),
            "learn" => Some(Self::Learn),
            _ => None,
        }
    }

    /// The canonical mode name written to config and shown by health.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
            Self::Learn => "learn",
        }
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    /// Shown to the agent in the tool result.
    pub reason: &'static str,
}

/// Server-side access check. Prompt text is not the control.
///
/// # Errors
///
/// Returns [`Denied`] when the access mode does not allow this actor and intent.
pub fn authorize(access: Access, actor: Actor, intent: Intent) -> Result<(), Denied> {
    if actor == Actor::Human {
        return Ok(());
    }
    match (access, intent) {
        (Access::Off, _) => Err(Denied {
            reason: "memory is off",
        }),
        (Access::ReadOnly, Intent::Read)
        | (Access::ReadWrite, Intent::Read | Intent::ExplicitWrite)
        | (Access::Learn, _) => Ok(()),
        (Access::ReadOnly, _) => Err(Denied {
            reason: "memory is read only",
        }),
        (Access::ReadWrite, Intent::AutomaticWrite) => Err(Denied {
            reason: "automatic writes are disabled",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_can_delete_when_agent_cannot() {
        assert!(authorize(Access::ReadOnly, Actor::Human, Intent::ExplicitWrite).is_ok());
        assert!(authorize(Access::ReadOnly, Actor::Agent, Intent::ExplicitWrite).is_err());
        assert!(authorize(Access::Off, Actor::Agent, Intent::Read).is_err());
        assert!(authorize(Access::ReadWrite, Actor::Agent, Intent::AutomaticWrite).is_err());
        assert!(authorize(Access::ReadWrite, Actor::Agent, Intent::ExplicitWrite).is_ok());
        assert!(authorize(Access::Learn, Actor::Agent, Intent::AutomaticWrite).is_ok());
    }
}
