#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use alicent_domain::*;
use alicent_project_repository::Repository;
use std::sync::{Arc, Mutex};
use tauri::State;
use uuid::Uuid;

#[derive(Clone, Default)]
struct AppState(Arc<Mutex<Option<Repository>>>);
type Reply<T> = Result<T, String>;

async fn with_repo<T: Send + 'static>(
    state: &AppState,
    action: impl FnOnce(&mut Repository) -> alicent_project_repository::Result<T> + Send + 'static,
) -> Reply<T> {
    let state = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "Состояние проекта недоступно".to_string())?;
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
        *state.0.lock().map_err(|_| "Состояние проекта недоступно")? = Some(repo);
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
        *state.0.lock().map_err(|_| "Состояние проекта недоступно")? = Some(repo);
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
) -> Reply<Document> {
    with_repo(&state, move |r| r.create_document(&title, kind, parent)).await
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
fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            create_project,
            open_project,
            list_documents,
            create_document,
            read_document,
            save_document,
            search_documents,
            list_versions,
            version_content,
            restore_version
        ])
        .run(tauri::generate_context!())
        .expect("Не удалось запустить Alicent");
}
