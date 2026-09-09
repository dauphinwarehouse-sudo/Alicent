import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ProjectPort } from "@alicent/contracts";
export const desktopAvailable = isTauri();
export const api: ProjectPort = {
  backupProject: () => invoke("backup_project"),
  restoreBackup: () => invoke("restore_backup"),
  cancelRecovery: () => invoke("cancel_recovery"),
  createCheckpoint: (id, name) => invoke("create_checkpoint", { id, name }),
  checkpoints: (offset = 0) => invoke("list_checkpoints", { offset }),
  checkpointPreview: (id, offset = 0) =>
    invoke("checkpoint_preview", { id, offset }),
  restoreCheckpoint: (id, expectedRevision, commandId) =>
    invoke("restore_checkpoint", { id, expectedRevision, commandId }),
  createProject: (title) => invoke("create_project", { title }),
  openProject: () => invoke("open_project"),
  list: (parent, offset = 0) => invoke("list_documents", { parent, offset }),
  createDocument: (title, kind, parent) =>
    invoke("create_document", { title, kind, parent }),
  renameDocument: (id, title, expectedRevision, commandId) =>
    invoke("rename_document", { id, title, expectedRevision, commandId }),
  duplicateDocument: (id, title, parent, commandId) =>
    invoke("duplicate_document", { id, title, parent, commandId }),
  read: (id) => invoke("read_document", { id }),
  save: (command) => invoke("save_document", { command }),
  search: (query) => invoke("search_documents", { query }),
  versions: (id, offset = 0) => invoke("list_versions", { id, offset }),
  versionContent: (id, revision) => invoke("version_content", { id, revision }),
  restore: (id, revision, expectedRevision, commandId) =>
    invoke("restore_version", { id, revision, expectedRevision, commandId }),
};
