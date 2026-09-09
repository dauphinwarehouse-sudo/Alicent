import { Schema, type Node as ProseMirrorNode } from "prosemirror-model";
import {
  defaultMarkdownParser,
  defaultMarkdownSerializer,
  MarkdownParser,
} from "prosemirror-markdown";
import { schema as basicSchema } from "prosemirror-schema-basic";
import { addListNodes } from "prosemirror-schema-list";

const nodes = addListNodes(
  basicSchema.spec.nodes,
  "paragraph block*",
  "block",
).update("doc", {
  content: "block+",
  attrs: {
    sourceMarkdown: { default: null },
    sourceFingerprint: { default: null },
  },
});

export const richTextSchema = new Schema({
  nodes,
  marks: basicSchema.spec.marks,
});

const markdownParser = new MarkdownParser(
  richTextSchema,
  defaultMarkdownParser.tokenizer,
  defaultMarkdownParser.tokens,
);

export function canonicalMarkdown(doc: ProseMirrorNode): string {
  return defaultMarkdownSerializer.serialize(doc);
}

export function parseMarkdown(markdown: string): ProseMirrorNode {
  const parsed = markdownParser.parse(markdown);
  if (!parsed) throw new Error("Markdown parser did not produce a document");
  return parsed.type.create(
    {
      sourceMarkdown: markdown,
      sourceFingerprint: canonicalMarkdown(parsed),
    },
    parsed.content,
  );
}

export function serializeMarkdown(doc: ProseMirrorNode): string {
  const canonical = canonicalMarkdown(doc);
  const original = doc.attrs.sourceMarkdown;
  const fingerprint = doc.attrs.sourceFingerprint;
  return typeof original === "string" && fingerprint === canonical
    ? original
    : canonical;
}

export function isExactSourcePreserved(doc: ProseMirrorNode): boolean {
  return (
    typeof doc.attrs.sourceMarkdown === "string" &&
    doc.attrs.sourceFingerprint === canonicalMarkdown(doc)
  );
}

const unsupportedPatterns: Array<[RegExp, string]> = [
  [/^---\r?\n/m, "YAML front matter остаётся только в lossless envelope"],
  [/<!--|<\/?[a-z][^>]*>/i, "HTML-блоки не отображаются выбранной схемой"],
  [/^\s*\|.+\|\s*$/m, "GFM-таблицы не входят в схему spike"],
  [/^\s*- \[[ xX]\]/m, "Task list отображается как обычный список"],
  [/~~[^~]+~~/, "Зачёркивание не входит в basic schema"],
];

export function supportWarnings(markdown: string): string[] {
  return unsupportedPatterns
    .filter(([pattern]) => pattern.test(markdown))
    .map(([, warning]) => warning);
}
