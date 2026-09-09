use std::fmt;
use zeroize::{Zeroize, Zeroizing};

#[cfg(windows)]
const SERVICE: &str = "com.alicent.provider";
const MAX_SECRET_BYTES: usize = 4096;

/// In-memory secret with zeroizing drop and deliberately redacted formatting.
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Result<Self, VaultError> {
        if value.is_empty() || value.len() > MAX_SECRET_BYTES || value.contains(['\r', '\n', '\0'])
        {
            return Err(VaultError::InvalidSecret);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretString([REDACTED])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    #[error("credential identifier is invalid")]
    InvalidId,
    #[error("credential value is invalid")]
    InvalidSecret,
    #[error("credential was not found")]
    NotFound,
    #[error("system credential vault is unavailable")]
    Unavailable,
    #[error("system credential vault operation failed")]
    OperationFailed,
}

pub trait CredentialVault: Send + Sync {
    fn store(&self, provider_id: &str, secret: SecretString) -> Result<(), VaultError>;
    fn load(&self, provider_id: &str) -> Result<SecretString, VaultError>;
    fn delete(&self, provider_id: &str) -> Result<(), VaultError>;
}

/// Production vault. On Windows this is backed by Windows Credential Manager.
/// Other platforms fail closed instead of falling back to a file or database.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsCredentialManager;

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(windows)]
fn entry(provider_id: &str) -> Result<keyring::Entry, VaultError> {
    if !valid_id(provider_id) {
        return Err(VaultError::InvalidId);
    }
    keyring::Entry::new(SERVICE, provider_id).map_err(|_| VaultError::Unavailable)
}

#[cfg(windows)]
impl CredentialVault for WindowsCredentialManager {
    fn store(&self, provider_id: &str, secret: SecretString) -> Result<(), VaultError> {
        entry(provider_id)?
            .set_password(secret.expose())
            .map_err(|_| VaultError::OperationFailed)
    }

    fn load(&self, provider_id: &str) -> Result<SecretString, VaultError> {
        let password = entry(provider_id)?
            .get_password()
            .map_err(|error| match error {
                keyring::Error::NoEntry => VaultError::NotFound,
                _ => VaultError::OperationFailed,
            })?;
        SecretString::new(password)
    }

    fn delete(&self, provider_id: &str) -> Result<(), VaultError> {
        entry(provider_id)?
            .delete_credential()
            .map_err(|error| match error {
                keyring::Error::NoEntry => VaultError::NotFound,
                _ => VaultError::OperationFailed,
            })
    }
}

#[cfg(not(windows))]
impl CredentialVault for WindowsCredentialManager {
    fn store(&self, provider_id: &str, _secret: SecretString) -> Result<(), VaultError> {
        if !valid_id(provider_id) {
            return Err(VaultError::InvalidId);
        }
        Err(VaultError::Unavailable)
    }

    fn load(&self, provider_id: &str) -> Result<SecretString, VaultError> {
        if !valid_id(provider_id) {
            return Err(VaultError::InvalidId);
        }
        Err(VaultError::Unavailable)
    }

    fn delete(&self, provider_id: &str) -> Result<(), VaultError> {
        if !valid_id(provider_id) {
            return Err(VaultError::InvalidId);
        }
        Err(VaultError::Unavailable)
    }
}
