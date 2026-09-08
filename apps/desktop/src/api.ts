import { invoke, isTauri } from "@tauri-apps/api/core";
import type { ProjectPort } from "@alicent/contracts";
export const desktopAvailable = isTauri();
export const api: ProjectPort = {
  createProject: (title) => invoke("create_project", { title }),
  openProject: () => invoke("open_project"),
  list: (parent, offset = 0) => invoke("list_documents", { parent, offset }),
  createDocument: (title, kind, parent) =>
    invoke("create_document", { title, kind, parent }),
  read: (id) => invoke("read_document", { id }),
  save: (command) => invoke("save_document", { command }),
  search: (query) => invoke("search_documents", { query }),
  versions: (id, offset = 0) => invoke("list_versions", { id, offset }),
  versionContent: (id, revision) => invoke("version_content", { id, revision }),
  restore: (id, revision, expectedRevision, commandId) =>
    invoke("restore_version", { id, revision, expectedRevision, commandId }),
};
