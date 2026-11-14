use serde::{Deserialize, Serialize};

/// Who is calling, and whether a write was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Agent,
    Human,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Read,
    ExplicitWrite,
    AutomaticWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Off,
    ReadOnly,
    ReadWrite,
    Learn,
}

impl Access {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "read_only" | "readonly" | "read-only" => Some(Self::ReadOnly),
            "read_write" | "readwrite" | "read-write" => Some(Self::ReadWrite),
            "learn" => Some(Self::Learn),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
            Self::Learn => "learn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub reason: &'static str,
}

/// Server-side access check. Prompt text is not the control.
pub fn authorize(access: Access, actor: Actor, intent: Intent) -> Result<(), Denied> {
    if actor == Actor::Human {
        return Ok(());
    }
    match (access, intent) {
        (Access::Off, _) => Err(Denied {
            reason: "memory is off",
        }),
        (Access::ReadOnly, Intent::Read) => Ok(()),
        (Access::ReadOnly, _) => Err(Denied {
            reason: "memory is read only",
        }),
        (Access::ReadWrite, Intent::AutomaticWrite) => Err(Denied {
            reason: "automatic writes are disabled",
        }),
        (Access::ReadWrite, _) => Ok(()),
        (Access::Learn, _) => Ok(()),
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
