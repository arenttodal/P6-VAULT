use serde::Serialize;

/// Structured error for the renderer: a stable code, a plain explanation and a next action.
#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub action: Option<String>,
    pub detail: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into(), action: default_action(code), detail: None }
    }
}

fn default_action(code: &str) -> Option<String> {
    Some(
        match code {
            "Offline" => "Connect the Prophet-6 (or enable Simulator mode) and try again.",
            "Busy" => "Wait for the current operation to finish or stop it.",
            "DeviceUnresponsive" => "Check the cable, the Prophet's Globals > MIDI SysEx port setting, and that no other librarian is running.",
            "Disconnected" => "Reconnect the synth. Nothing further was written.",
            "RevisionConflict" => "The view was refreshed; repeat the action.",
            "IncompleteBank" => "Retry the missing slots or fill the empty slots.",
            "HardwareDrift" => "Review the differences and choose Keep New or Use Synth.",
            "BackupFailed" => "Check free disk space and permissions. No programs were written.",
            "JournalFailed" => "Check free disk space. Writing stopped safely.",
            "SelectionOverflow" => "Choose an earlier start slot.",
            "HardwareGate" => "Stage exactly one change in a slot you have chosen for testing, write it, then restore it (see docs/HARDWARE-TESTS.md).",
            _ => return None,
        }
        .into(),
    )
}

impl From<p6_core::storage::VaultError> for ApiError {
    fn from(e: p6_core::storage::VaultError) -> Self {
        use p6_core::storage::VaultError as V;
        let code = match &e {
            V::RevisionConflict { .. } => "RevisionConflict",
            V::IncompleteBank(_) => "IncompleteBank",
            V::Operation(p6_core::workspace::operations::OpError::SelectionOverflow { .. }) => "SelectionOverflow",
            V::Operation(_) => "InvalidDestination",
            V::NotFound(_) => "NotFound",
            V::Unsupported(_) => "UnsupportedFormat",
            V::Invalid(_) => "Invalid",
            V::Database(_) | V::Io(_) => "StorageError",
        };
        let mut a = ApiError::new(code, e.to_string());
        a.detail = serde_json::to_value(&e).ok();
        a
    }
}

impl From<p6_core::deployment::DeployError> for ApiError {
    fn from(e: p6_core::deployment::DeployError) -> Self {
        use p6_core::deployment::DeployError as D;
        match e {
            D::Vault(v) => v.into(),
            D::Device(d) => d.into(),
            other => {
                let code = match &other {
                    D::IncompleteBank { .. } => "IncompleteBank",
                    D::HardwareDrift { .. } => "HardwareDrift",
                    D::BackupFailed(_) => "BackupFailed",
                    D::JournalFailed(_) => "JournalFailed",
                    D::VerificationMismatch { .. } => "VerificationMismatch",
                    D::InvalidPermit(_) => "InvalidPermit",
                    D::Cancelled => "Cancelled",
                    D::HardwareGate { .. } => "HardwareGate",
                    _ => "Invalid",
                };
                ApiError::new(code, other.to_string())
            }
        }
    }
}

impl From<p6_core::device::DeviceError> for ApiError {
    fn from(e: p6_core::device::DeviceError) -> Self {
        use p6_core::device::DeviceError as D;
        let code = match &e {
            D::DeviceUnresponsive { .. } | D::NotAProphet6(_) => "DeviceUnresponsive",
            D::Disconnected => "Disconnected",
            D::QueueOverflow => "QueueOverflow",
            D::Cancelled => "Cancelled",
            D::Io(_) => "MidiError",
            D::Refused(_) => "Refused",
        };
        ApiError::new(code, e.to_string())
    }
}

impl From<p6_core::library::export::ExportError> for ApiError {
    fn from(e: p6_core::library::export::ExportError) -> Self {
        use p6_core::library::export::ExportError as E;
        let code = match &e {
            E::IncompleteBank(_) => "IncompleteBank",
            E::DuplicateDestination(_) => "InvalidDestination",
            _ => "ExportFailed",
        };
        ApiError::new(code, e.to_string())
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
