/** Test-only in-memory IPC double. This is NOT an application entry point or storage adapter. */
import { createRoot } from "react-dom/client";
import type { Document, ProjectPort } from "@alicent/contracts";
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
const histories = new Map<string, Document[]>([[initial.id, [initial]]]);
const project = {
  id: "test-project",
  title: "Хроники северного берега",
  schema_version: 1,
  created_at: initial.updated_at,
};
const port: ProjectPort = {
  async createProject(title) {
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
    docs.set(doc.id, doc);
    histories.set(doc.id, [doc]);
    return doc;
  },
  async read(id) {
    return { ...docs.get(id)! };
  },
  async save(cmd) {
    const old = docs.get(cmd.document_id)!;
    if (old.revision !== cmd.expected_revision) throw "Конфликт версий";
    const next = { ...old, content: cmd.content, revision: old.revision + 1 };
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
