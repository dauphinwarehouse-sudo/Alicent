# ProseMirror rich-text spike (P0.2)

Изолированное Vite-приложение. Оно не импортируется production desktop app и не заменяет `apps/desktop/src/Editor.tsx` (CodeMirror).

## Запуск

```bash
npm ci
npm run spike:rich-text
```

Открыть `http://127.0.0.1:1421`. Кнопка «Замерить 200k слов» показывает parse/state, первый animation frame после `updateState` и serialize для сгенерированного кириллического документа.

## Проверки

```bash
npm test
npm run bench:rich-text
npm run build -w @alicent/rich-text-spike
```

`bench:rich-text` запускает тот же latency test на 200 000 слов. CI также использует 200 000 слов и мягкий ceiling 5 s, чтобы ловить грубые регрессии.

## Что считается lossless

Корневой ProseMirror `doc` хранит исходный Markdown и fingerprint его канонического представления. Пока содержимое block tree не менялось — включая JSON persistence и undo к исходному состоянию — serializer возвращает исходную строку байт-в-байт (CRLF, кириллица, emoji и неподдержанные конструкции включительно).

После rich-text правки serializer намеренно выдаёт канонический Markdown ProseMirror. Значит, произвольный Markdown пока нельзя автоматически мигрировать в WYSIWYG: front matter, HTML, GFM table/task list и strikethrough требуют raw block/mark extensions либо блокировки миграции.
