use alicent_provider_wire::{
    provider_capabilities as capabilities, AbortHandle, ProviderCapabilities, ProviderCommandError,
    ProviderConnectionResult, ProviderErrorCode, ProviderGenerationRequest,
    ProviderGenerationResult, ProviderKind, ProviderReply, ProviderRuntime, ProviderSettingsDraft,
    ProviderSettingsSnapshot, WindowsCredentialManager,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{ipc::Channel, State};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct ProviderState {
    runtime: ProviderRuntime<WindowsCredentialManager>,
    active: Arc<Mutex<HashMap<Uuid, AbortHandle>>>,
}

impl ProviderState {
    pub(crate) fn new(settings_path: PathBuf) -> Self {
        Self {
            runtime: ProviderRuntime::new(settings_path, Arc::new(WindowsCredentialManager)),
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ProviderStreamEvent {
    Delta { text: String },
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

#[tauri::command]
pub(crate) async fn generate_provider_text(
    state: State<'_, ProviderState>,
    request_id: Uuid,
    request: ProviderGenerationRequest,
    on_event: Channel<ProviderStreamEvent>,
) -> ProviderReply<ProviderGenerationResult> {
    let abort = AbortHandle::default();
    {
        let mut active = state
            .active
            .lock()
            .map_err(|_| ProviderCommandError::from_code(ProviderErrorCode::SettingsUnavailable))?;
        match active.entry(request_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(abort.clone());
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                return Err(ProviderCommandError::from_code(
                    ProviderErrorCode::InvalidSettings,
                ));
            }
        }
    }

    let result = state
        .runtime
        .clone()
        .generate_text(request, abort, move |text| {
            let _ = on_event.send(ProviderStreamEvent::Delta {
                text: text.to_owned(),
            });
        })
        .await;

    state
        .active
        .lock()
        .map_err(|_| ProviderCommandError::from_code(ProviderErrorCode::SettingsUnavailable))?
        .remove(&request_id);
    result
}

#[tauri::command]
pub(crate) fn cancel_provider_generation(
    state: State<'_, ProviderState>,
    request_id: Uuid,
) -> ProviderReply<()> {
    let abort = state
        .active
        .lock()
        .map_err(|_| ProviderCommandError::from_code(ProviderErrorCode::SettingsUnavailable))?
        .get(&request_id)
        .cloned();
    if let Some(abort) = abort {
        abort.abort();
    }
    Ok(())
}
