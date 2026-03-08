use thiserror::Error;

pub type Result<T> = std::result::Result<T, VoxioError>;

#[derive(Debug, Error)]
pub enum VoxioError {
    #[error("{0}")]
    Validation(String),
    #[error("input injection failed: {0}")]
    Injection(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl From<VoxioError> for tauri::ipc::InvokeError {
    fn from(value: VoxioError) -> Self {
        tauri::ipc::InvokeError::from(value.to_string())
    }
}
