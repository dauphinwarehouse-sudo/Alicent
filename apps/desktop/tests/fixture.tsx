/** Test-only in-memory IPC double. This is NOT an application entry point or storage adapter. */
import { createRoot } from "react-dom/client";
import type { Checkpoint, Document, ProjectPort } from "@alicent/contracts";
import { App } from "../src/App";
import "../src/style.css";
const initial: Document = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "01. Северный ветер",
  kind: "scene",
  parent_id: null,
  revision: 0,
  updated_at: "2026-09-01T08:30:00Z",
  content:
    "# Северный ветер\n\nВ день, когда море отступило, Элина впервые услышала колокола затонувшего города. Их звук не был похож на звон — скорее на дыхание, медленное и глубокое, будто кто-то просыпался под толщей воды.\n\nОна стояла у старого маяка и держала письмо, которое не решалась открыть уже три дня. Бумага пахла солью и немного — дымом.\n\n«Если ветер переменится, — говорил отец, — не закрывай окна».\n\nВетер переменился ночью.",
};
const docs = new Map<string, Document>([[initial.id, initial]]);
const archivedDocs = new Map<
  string,
  Document & { archived_at: string; affected_count: number }
>();
const histories = new Map<string, Document[]>([[initial.id, [initial]]]);
const project = {
  id: "test-project",
  title: "Хроники северного берега",
  schema_version: 3,
  created_at: initial.updated_at,
};
let projectRevision = 0;
const checkpoints = new Map<string, { info: Checkpoint; docs: Document[] }>();
const port: ProjectPort = {
  async backupProject() { return { path: "TEST-ONLY/копия.alicent-backup", bytes: 4096, schema_version: 3 }; },
  async restoreBackup() { return { ...project, title: "Восстановленная рукопись" }; },
  async cancelRecovery() {},
  async createCheckpoint(id, name) {
    const snapshot = [...docs.values()].filter(d => d.kind !== "folder").map(d => ({...d}));
    const info = { id, name, created_at: new Date().toISOString(), document_count: snapshot.length };
    checkpoints.set(id, {info, docs: snapshot}); return info;
  },
  async checkpoints(offset = 0) { return [...checkpoints.values()].reverse().slice(offset, offset + 200).map(c => c.info); },
  async checkpointPreview(id, offset = 0) {
    const cp = checkpoints.get(id)!;
    const rows = cp.docs.map(d => ({id: d.id, title: d.title, current_revision: docs.get(d.id)!.revision, target_revision: d.revision, changed: docs.get(d.id)!.content !== d.content }));
    return { checkpoint: cp.info, project_revision: projectRevision, changed_count: rows.filter(d => d.changed).length,
      newer_document_count: [...docs.values()].filter(d => d.kind !== "folder" && !cp.docs.some(old => old.id === d.id)).length,
      documents: rows.slice(offset, offset + 200), has_more: rows.length > offset + 200 };
  },
  async restoreCheckpoint(id, expected, commandId) {
    if (expected !== projectRevision) throw "Проект изменился. Откройте предпросмотр заново.";
    const cp = checkpoints.get(id)!;
    const changed = cp.docs.filter(d => docs.get(d.id)!.content !== d.content);
    const undo = changed.length ? await port.createCheckpoint(crypto.randomUUID(), `Перед восстановлением: ${cp.info.name}`) : null;
    for (const doc of changed) await port.save({document_id: doc.id, content: doc.content, expected_revision: docs.get(doc.id)!.revision, command_id: crypto.randomUUID()});
    return {checkpoint_id: id, changed_count: changed.length, operation_id: commandId, undo_checkpoint_id: undo?.id ?? null};
  },
  async createProject(title) {
    checkpoints.clear();
    projectRevision++;
    docs.clear();
    histories.clear();
    return { ...project, title };
  },
  async openProject() {
    return project;
  },
  async list(parent, offset = 0) {
    return [...docs.values()]
      .filter((d) => d.parent_id === parent)
      .slice(offset, offset + 200);
  },
  async createDocument(title, kind, parent) {
    const doc = {
      ...initial,
      id: crypto.randomUUID(),
      title,
      kind,
      parent_id: parent,
      content: "",
    };
    projectRevision++;
    docs.set(doc.id, doc);
    histories.set(doc.id, [doc]);
    return doc;
  },
  async renameDocument(id, title, expectedRevision) {
    const old = docs.get(id)!;
    if (old.revision !== expectedRevision) throw "Конфликт версий";
    const next = { ...old, title, revision: old.revision + 1 };
    docs.set(id, next);
    histories.get(id)!.push(next);
    return { ...next };
  },
  async duplicateDocument(id, title, parent) {
    const old = docs.get(id)!;
    const copy = {
      ...old,
      id: crypto.randomUUID(),
      title,
      parent_id: parent,
      revision: 0,
    };
    docs.set(copy.id, copy);
    histories.set(copy.id, [copy]);
    return { ...copy };
  },
  async archived(offset = 0) {
    return [...archivedDocs.values()].slice(offset, offset + 200);
  },
  async archiveDocument(command) {
    const document = docs.get(command.document_id)!;
    docs.delete(command.document_id);
    archivedDocs.set(command.document_id, {
      ...document,
      archived_at: new Date().toISOString(),
      affected_count: 1,
    });
    return { ...command, affected_count: 1 };
  },
  async restoreArchived(command) {
    const document = archivedDocs.get(command.document_id)!;
    archivedDocs.delete(command.document_id);
    docs.set(command.document_id, document);
    return { ...command, affected_count: 1 };
  },
  async moveDocument(command) {
    const document = docs.get(command.document_id)!;
    if (document.revision !== command.expected_revision)
      throw "Конфликт версий";
    if (
      command.parent_id &&
      (!docs.has(command.parent_id) ||
        docs.get(command.parent_id)!.kind !== "folder")
    )
      throw "Недопустимый родитель";
    const moved = {
      ...document,
      parent_id: command.parent_id,
      revision: document.revision + 1,
    };
    docs.set(document.id, moved);
    histories.get(document.id)!.push(moved);
    return { ...moved };
  },
  async read(id) {
    return { ...docs.get(id)! };
  },
  async save(cmd) {
    // Test-only fault injection; fixture.html is not a production entry point.
    if (new URLSearchParams(location.search).has("save-error")) {
      throw "Тестовая ошибка сохранения. Черновик остаётся в редакторе.";
    }
    const old = docs.get(cmd.document_id)!;
    if (old.revision !== cmd.expected_revision) throw "Конфликт версий";
    const next = { ...old, content: cmd.content, revision: old.revision + 1 };
    projectRevision++;
    docs.set(old.id, next);
    histories.get(old.id)!.push(next);
    return { ...next };
  },
  async search(query) {
    return [...docs.values()].filter((d) =>
      (d.title + d.content).includes(query),
    );
  },
  async versions(id, offset = 0) {
    return [...histories.get(id)!]
      .reverse()
      .slice(offset, offset + 200)
      .map((d) => ({
        revision: d.revision,
        created_at: d.updated_at,
        actor: "user:local",
      }));
  },
  async versionContent(id, revision) {
    return histories.get(id)!.find((d) => d.revision === revision)!.content;
  },
  async restore(id, revision, expectedRevision, commandId) {
    return port.save({
      document_id: id,
      expected_revision: expectedRevision,
      command_id: commandId,
      content: await port.versionContent(id, revision),
    });
  },
};
createRoot(document.getElementById("root")!).render(
  <App port={port} available />,
);
