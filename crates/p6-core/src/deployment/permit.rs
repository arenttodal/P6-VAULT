//! Typed write permission. A [`ConfirmedWritePermit`] can only be constructed in this
//! crate's deployment module after the user confirms a frozen plan, and is bound to
//! that plan's hash, the workspace revision and the connection epoch.

#[derive(Debug)]
pub struct ConfirmedWritePermit {
    pub(crate) session_id: String,
    pub(crate) plan_hash: String,
    pub(crate) workspace_revision: i64,
    pub(crate) epoch: u64,
}

impl ConfirmedWritePermit {
    pub(crate) fn new(session_id: String, plan_hash: String, workspace_revision: i64, epoch: u64) -> Self {
        Self { session_id, plan_hash, workspace_revision, epoch }
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }
    pub fn workspace_revision(&self) -> i64 {
        self.workspace_revision
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}
