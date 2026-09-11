#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod provider_commands;

use alicent_domain::*;
use alicent_project_repository::Repository;
use provider_commands::{
    cancel_provider_generation, delete_provider_credential, generate_provider_text,
    load_provider_settings, provider_capabilities, save_provider_settings,
    store_provider_credential, test_provider_connection, ProviderState,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};
use tauri::{Manager, State};
use uuid::Uuid;

#[derive(Clone, Default)]
struct AppState {
    repo: Arc<Mutex<Option<Repository>>>,
    recovery: RecoveryCancellation,
}
type Reply<T> = Result<T, String>;

/// Cancellation flags for the long recovery operations: backup, restore and
/// checkpoint restore.
///
/// A single shared flag served all of them, so starting a second operation
/// cleared the flag the first one was still watching, which silently dropped
/// a cancel the user had already confirmed, and one cancel reached into
/// operations the user never cancelled. Each operation now registers its own
/// flag and a cancel raises every flag currently running, which keeps the
/// no-argument command the UI calls unchanged.
#[derive(Clone, Default)]
struct RecoveryCancellation {
    flags: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
}

/// Registration of one running operation. Dropping it unregisters the flag,
/// so a cancel arriving after the operation has finished does nothing.
struct RecoveryToken {
    flag: Arc<AtomicBool>,
    flags: Arc<Mutex<Vec<Arc<AtomicBool>>>>,
}

impl RecoveryCancellation {
    fn begin(&self) -> RecoveryToken {
        let flag = Arc::new(AtomicBool::new(false));
        self.guard().push(flag.clone());
        RecoveryToken {
            flag,
            flags: self.flags.clone(),
        }
    }

    fn cancel_all(&self) {
        for flag in self.guard().iter() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// A panic cannot leave a list of flags inconsistent, so recovering from
    /// a poisoned lock is safe here and beats refusing every later cancel.
    fn guard(&self) -> MutexGuard<'_, Vec<Arc<AtomicBool>>> {
        self.flags.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl RecoveryToken {
    fn flag(&self) -> &Arc<AtomicBool> {
        &self.flag
    }
}

impl Drop for RecoveryToken {
    fn drop(&mut self) {
        let mut flags = self.flags.lock().unwrap_or_else(|e| e.into_inner());
        flags.retain(|flag| !Arc::ptr_eq(flag, &self.flag));
    }
}

/// The lock guards an optional repository handle, and an unfinished SQLite
/// transaction is rolled back while a panic unwinds, so recovering from
/// poisoning leaves the project usable. Refusing every later command because
/// one earlier command panicked bricked the whole session instead.
fn lock_repo(state: &AppState) -> MutexGuard<'_, Option<Repository>> {
    state.repo.lock().unwrap_or_else(|e| e.into_inner())
}

async fn with_repo<T: Send + 'static>(
    state: &AppState,
    action: impl FnOnce(&mut Repository) -> alicent_project_repository::Result<T> + Send + 'static,
) -> Reply<T> {
    let state = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = lock_repo(&state);
        let repo = guard.as_mut().ok_or("Сначала откройте проект")?;
        action(repo).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Операция прервана".to_string())?
}
#[tauri::command]
async fn create_project(state: State<'_, AppState>, title: String) -> Reply<Option<Project>> {
    validate_title(&title).map_err(|e| e.to_string())?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(parent) = rfd::FileDialog::new()
            .set_title("Каталог для нового проекта Alicent")
            .pick_folder()
        else {
            return Ok(None);
        };
        let repo = Repository::create(&parent, &title).map_err(|e| e.to_string())?;
        let project = repo.project().map_err(|e| e.to_string())?;
        *lock_repo(&state) = Some(repo);
        Ok(Some(project))
    })
    .await
    .map_err(|_| "Операция прервана".to_string())?
}
#[tauri::command]
async fn open_project(state: State<'_, AppState>) -> Reply<Option<Project>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(root) = rfd::FileDialog::new()
            .set_title("Откройте папку .alicent с project.sqlite3")
            .pick_folder()
        else {
            return Ok(None);
        };
        let repo = Repository::open(&root).map_err(|e| e.to_string())?;
        let project = repo.project().map_err(|e| e.to_string())?;
        *lock_repo(&state) = Some(repo);
        Ok(Some(project))
    })
    .await
    .map_err(|_| "Операция прервана".to_string())?
}
#[tauri::command]
async fn list_documents(
    state: State<'_, AppState>,
    parent: Option<Uuid>,
    offset: u32,
) -> Reply<Vec<DocumentSummary>> {
    with_repo(&state, move |r| r.list(parent, 200, offset)).await
}
#[tauri::command]
async fn create_document(
    state: State<'_, AppState>,
    title: String,
    kind: DocumentKind,
    parent: Option<Uuid>,
    command_id: Option<Uuid>,
) -> Reply<Document> {
    let command_id = command_id.unwrap_or_else(Uuid::new_v4);
    with_repo(&state, move |r| {
        r.create_document_with_operation_id(command_id, &title, kind, parent)
    })
    .await
}
#[tauri::command]
async fn rename_document(
    state: State<'_, AppState>,
    id: Uuid,
    title: String,
    expected_revision: i64,
    command_id: Uuid,
) -> Reply<Document> {
    with_repo(&state, move |r| {
        r.rename_document(id, &title, expected_revision, command_id)
    })
    .await
}
#[tauri::command]
async fn duplicate_document(
    state: State<'_, AppState>,
    id: Uuid,
    title: String,
    parent: Option<Uuid>,
    command_id: Uuid,
) -> Reply<Document> {
    with_repo(&state, move |r| {
        r.duplicate_document(id, &title, parent, command_id)
    })
    .await
}
#[tauri::command]
async fn list_archived(state: State<'_, AppState>, offset: u32) -> Reply<Vec<ArchivedDocument>> {
    with_repo(&state, move |r| r.archived(200, offset)).await
}
#[tauri::command]
async fn archive_document(
    state: State<'_, AppState>,
    command: ArchiveDocument,
) -> Reply<ArchiveReceipt> {
    with_repo(&state, move |r| r.archive_document(command)).await
}
#[tauri::command]
async fn restore_archived(
    state: State<'_, AppState>,
    command: ArchiveDocument,
) -> Reply<ArchiveReceipt> {
    with_repo(&state, move |r| r.restore_archived(command)).await
}
#[tauri::command]
async fn move_document(state: State<'_, AppState>, command: MoveDocument) -> Reply<Document> {
    with_repo(&state, move |r| r.move_document(command)).await
}
#[tauri::command]
async fn pinned_ai_context(state: State<'_, AppState>) -> Reply<Vec<DocumentSummary>> {
    with_repo(&state, |r| r.pinned_ai_context()).await
}
#[tauri::command]
async fn set_document_ai_context(
    state: State<'_, AppState>,
    command: SetDocumentAiContext,
) -> Reply<Document> {
    with_repo(&state, move |r| r.set_document_ai_context(command)).await
}
#[tauri::command]
async fn read_document(state: State<'_, AppState>, id: Uuid) -> Reply<Document> {
    with_repo(&state, move |r| r.read(id)).await
}
#[tauri::command]
async fn save_document(state: State<'_, AppState>, command: SaveDocument) -> Reply<Document> {
    with_repo(&state, move |r| r.save(command)).await
}
#[tauri::command]
async fn search_documents(
    state: State<'_, AppState>,
    query: String,
) -> Reply<Vec<DocumentSummary>> {
    if query.len() > 2048 {
        return Err("Поисковый запрос слишком длинный".into());
    }
    with_repo(&state, move |r| r.search(&query, 200)).await
}
#[tauri::command]
async fn list_versions(
    state: State<'_, AppState>,
    id: Uuid,
    offset: u32,
) -> Reply<Vec<VersionSummary>> {
    with_repo(&state, move |r| r.versions(id, 200, offset)).await
}
#[tauri::command]
async fn version_content(state: State<'_, AppState>, id: Uuid, revision: i64) -> Reply<String> {
    with_repo(&state, move |r| r.version_content(id, revision)).await
}
#[tauri::command]
async fn restore_version(
    state: State<'_, AppState>,
    id: Uuid,
    revision: i64,
    expected_revision: i64,
    command_id: Uuid,
) -> Reply<Document> {
    with_repo(&state, move |r| {
        r.restore(id, revision, expected_revision, command_id)
    })
    .await
}
#[tauri::command]
fn cancel_recovery(state: State<'_, AppState>) {
    state.recovery.cancel_all();
}
#[tauri::command]
async fn backup_project(state: State<'_, AppState>) -> Reply<Option<BackupInfo>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(directory) = rfd::FileDialog::new()
            .set_title("Каталог для резервной копии Alicent")
            .pick_folder()
        else {
            return Ok(None);
        };
        let cancel = state.recovery.begin();
        let guard = lock_repo(&state);
        let repo = guard.as_ref().ok_or("Сначала откройте проект")?;
        repo.backup_to(&directory, cancel.flag())
            .map(Some)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Операция прервана".to_string())?
}
#[tauri::command]
async fn restore_backup(state: State<'_, AppState>) -> Reply<Option<Project>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(backup) = rfd::FileDialog::new()
            .set_title("Резервная копия Alicent")
            .add_filter("Alicent backup", &["alicent-backup"])
            .pick_file()
        else {
            return Ok(None);
        };
        let Some(parent) = rfd::FileDialog::new()
            .set_title("Каталог для восстановленного проекта (будет создана новая папка)")
            .pick_folder()
        else {
            return Ok(None);
        };
        let cancel = state.recovery.begin();
        let repo = Repository::restore_backup(&backup, &parent, cancel.flag())
            .map_err(|e| e.to_string())?;
        let project = repo.project().map_err(|e| e.to_string())?;
        *lock_repo(&state) = Some(repo);
        Ok(Some(project))
    })
    .await
    .map_err(|_| "Операция прервана".to_string())?
}
#[tauri::command]
async fn create_checkpoint(
    state: State<'_, AppState>,
    id: Uuid,
    name: String,
) -> Reply<Checkpoint> {
    with_repo(&state, move |r| r.create_checkpoint(id, &name)).await
}
#[tauri::command]
async fn list_checkpoints(state: State<'_, AppState>, offset: u32) -> Reply<Vec<Checkpoint>> {
    with_repo(&state, move |r| r.checkpoints(200, offset)).await
}
#[tauri::command]
async fn checkpoint_preview(
    state: State<'_, AppState>,
    id: Uuid,
    offset: u32,
) -> Reply<CheckpointPreview> {
    with_repo(&state, move |r| r.checkpoint_preview(id, 200, offset)).await
}
#[tauri::command]
async fn restore_checkpoint(
    state: State<'_, AppState>,
    id: Uuid,
    expected_revision: i64,
    command_id: Uuid,
) -> Reply<CheckpointRestore> {
    let cancel = state.recovery.begin();
    let flag = cancel.flag().clone();
    with_repo(&state, move |r| {
        r.restore_checkpoint_cancellable(id, expected_revision, command_id, &flag)
    })
    .await
}
fn main() {
    // A startup failure used to panic here. The release build sets
    // windows_subsystem = "windows", so that panic produced nothing a user
    // could act on; report the reason and exit with a failing status.
    let launched = tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("provider-settings.json");
            app.manage(ProviderState::new(settings_path));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_project,
            open_project,
            list_documents,
            create_document,
            rename_document,
            duplicate_document,
            list_archived,
            archive_document,
            restore_archived,
            move_document,
            pinned_ai_context,
            set_document_ai_context,
            read_document,
            save_document,
            search_documents,
            list_versions,
            version_content,
            restore_version,
            backup_project,
            restore_backup,
            cancel_recovery,
            create_checkpoint,
            list_checkpoints,
            checkpoint_preview,
            restore_checkpoint,
            provider_capabilities,
            load_provider_settings,
            save_provider_settings,
            store_provider_credential,
            delete_provider_credential,
            test_provider_connection,
            generate_provider_text,
            cancel_provider_generation
        ])
        .run(tauri::generate_context!());
    if let Err(error) = launched {
        eprintln!("Не удалось запустить Alicent: {error}");
        std::process::exit(1);
    }
}
