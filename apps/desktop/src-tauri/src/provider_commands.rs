use alicent_provider_wire::{
    provider_capabilities as capabilities, ProviderCapabilities, ProviderCommandError,
    ProviderConnectionResult, ProviderErrorCode, ProviderKind, ProviderReply, ProviderRuntime,
    ProviderSettingsDraft, ProviderSettingsSnapshot, WindowsCredentialManager,
};
use std::{path::PathBuf, sync::Arc};
use tauri::State;

#[derive(Clone)]
pub(crate) struct ProviderState {
    runtime: ProviderRuntime<WindowsCredentialManager>,
}

impl ProviderState {
    pub(crate) fn new(settings_path: PathBuf) -> Self {
        Self {
            runtime: ProviderRuntime::new(settings_path, Arc::new(WindowsCredentialManager)),
        }
    }
}

async fn blocking<T: Send + 'static>(
    action: impl FnOnce() -> ProviderReply<T> + Send + 'static,
) -> ProviderReply<T> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|_| ProviderCommandError::from_code(ProviderErrorCode::SettingsUnavailable))?
}

#[tauri::command]
pub(crate) fn provider_capabilities() -> ProviderCapabilities {
    capabilities()
}

#[tauri::command]
pub(crate) async fn load_provider_settings(
    state: State<'_, ProviderState>,
) -> ProviderReply<ProviderSettingsSnapshot> {
    let runtime = state.runtime.clone();
    blocking(move || runtime.load_settings()).await
}

#[tauri::command]
pub(crate) async fn save_provider_settings(
    state: State<'_, ProviderState>,
    settings: ProviderSettingsDraft,
) -> ProviderReply<ProviderSettingsSnapshot> {
    let runtime = state.runtime.clone();
    blocking(move || runtime.save_settings(settings)).await
}

#[tauri::command]
pub(crate) async fn store_provider_credential(
    state: State<'_, ProviderState>,
    provider: ProviderKind,
    secret: String,
) -> ProviderReply<()> {
    let runtime = state.runtime.clone();
    blocking(move || runtime.store_credential(provider, secret)).await
}

#[tauri::command]
pub(crate) async fn delete_provider_credential(
    state: State<'_, ProviderState>,
    provider: ProviderKind,
) -> ProviderReply<()> {
    let runtime = state.runtime.clone();
    blocking(move || runtime.delete_credential(provider)).await
}

#[tauri::command]
pub(crate) async fn test_provider_connection(
    state: State<'_, ProviderState>,
    settings: ProviderSettingsDraft,
) -> ProviderReply<ProviderConnectionResult> {
    state.runtime.clone().test_connection(settings).await
}
