use crate::{Result, WireError};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Untrusted proposal only. JSON Schema validation + runtime authorization are still mandatory.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolProposal {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
struct Pending {
    id: String,
    name: String,
    json: String,
}
#[derive(Default)]
pub struct ToolArguments {
    pending: BTreeMap<u32, Pending>,
    bytes: usize,
    closed: bool,
}
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}
impl ToolArguments {
    pub const MAX_CALLS: usize = 64;
    pub const MAX_ARGUMENTS: usize = 1024 * 1024;
    pub const MAX_TOTAL: usize = 4 * 1024 * 1024;
    pub fn start(&mut self, index: u32, id: &str, name: &str) -> Result<()> {
        if self.closed {
            return Err(WireError::Closed);
        }
        if self.pending.len() >= Self::MAX_CALLS
            || self.pending.contains_key(&index)
            || !valid_id(id)
            || !valid_name(name)
            || self.pending.values().any(|p| p.id == id)
        {
            self.cancel();
            return Err(WireError::InvalidArguments);
        }
        self.pending.insert(
            index,
            Pending {
                id: id.into(),
                name: name.into(),
                json: String::new(),
            },
        );
        Ok(())
    }
    pub fn append(&mut self, index: u32, fragment: &str) -> Result<()> {
        if self.closed {
            return Err(WireError::Closed);
        }
        let Some(pending) = self.pending.get_mut(&index) else {
            self.cancel();
            return Err(WireError::InvalidArguments);
        };
        if fragment.len() > Self::MAX_ARGUMENTS
            || pending.json.len() + fragment.len() > Self::MAX_ARGUMENTS
            || self.bytes + fragment.len() > Self::MAX_TOTAL
        {
            self.cancel();
            return Err(WireError::LimitExceeded);
        }
        pending.json.push_str(fragment);
        self.bytes += fragment.len();
        Ok(())
    }
    /// Emit nothing until the normalizer observes a successful protocol tool-call stop.
    /// Any invalid call rejects the whole batch. A length stop/cancel must call `cancel`.
    pub fn finish(&mut self, allowed_names: &BTreeSet<String>) -> Result<Vec<ToolProposal>> {
        if self.closed {
            return Err(WireError::Closed);
        }
        self.closed = true;
        let pending = std::mem::take(&mut self.pending);
        self.bytes = 0;
        pending
            .into_values()
            .map(|p| {
                if !allowed_names.contains(&p.name) {
                    return Err(WireError::InvalidArguments);
                }
                let arguments: Value =
                    serde_json::from_str(&p.json).map_err(|_| WireError::InvalidArguments)?;
                if !arguments.is_object() {
                    return Err(WireError::InvalidArguments);
                }
                Ok(ToolProposal {
                    id: p.id,
                    name: p.name,
                    arguments,
                })
            })
            .collect()
    }
    pub fn cancel(&mut self) {
        self.closed = true;
        self.pending.clear();
        self.bytes = 0;
    }
}
