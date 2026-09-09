# ADR 0009 — ProseMirror rich-text spike и путь миграции Markdown

Статус: принято для эксперимента; production-миграция отложена.

## Контекст

ADR 0003 оставил rich text обязательным исследованием, но production-редактор сейчас основан на CodeMirror 6 и Markdown. P0.2 должен проверить block schema, lossless round-trip, историю, кириллицу/IME и базовую стоимость большого документа, не меняя текущий editor path.

## Решение

Для spike выбран прямой ProseMirror, без TipTap. TipTap упрощает UI extensions, но не решает ключевые риски этого проекта: fidelity Markdown и рендер длинного документа. Прямая интеграция оставляет схему и сериализацию явными и добавляет меньше абстракций.

Spike живёт отдельным Vite workspace `apps/rich-text-spike`; production `apps/desktop/src/Editor.tsx` не импортирует его. Схема основана на `prosemirror-schema-basic` плюс list nodes: paragraph, heading, blockquote, code block, horizontal rule, ordered/bullet list, list item, image, hard break и text с strong/em/link/code marks.

Корень `doc` дополнен lossless envelope:

- `sourceMarkdown` хранит исходную строку;
- `sourceFingerprint` хранит её каноническое ProseMirror-представление;
- serializer возвращает исходную строку байт-в-байт, пока block tree равен fingerprint;
- после rich-text изменения serializer выдаёт канонический Markdown;
- undo к исходному tree снова возвращает исходную строку, включая CRLF.

Это даёт честный exact round-trip для открытия, JSON persistence и закрытия без правок. Это не даёт полного lossless редактирования произвольного Markdown: неподдержанная конструкция сохранится в envelope только пока пользователь не изменит rich-text tree.

## Результаты

Автотесты покрывают:

- exact round-trip кириллицы, emoji, CRLF, YAML front matter и HTML comment;
- JSON round-trip выбранной block schema;
- переход к каноническому Markdown после правки;
- undo/redo с восстановлением исходного Markdown;
- native `compositionstart` / `beforeinput(insertCompositionText)` без отмены события и commit кириллического текста;
- предупреждения для front matter, HTML, GFM table/task list и strikethrough.

Локальный baseline на Linux, Node 24, сгенерированный документ 200 000 слов: parse + EditorState ≈ 0.96 s, serialize ≈ 0.97 s. Это измерение CPU без layout/paint. UI spike отдельно измеряет первый `requestAnimationFrame` после `EditorView.updateState`; показатель зависит от машины и должен сниматься на целевом Windows WebView. CI smoke на 20 000 слов имеет мягкий ceiling 5 s для parse и serialize, чтобы ловить только грубые регрессии.

## Оценка технологии

ProseMirror подходит как основа ограниченного rich-text режима с контролируемой схемой. Он не подходит как немедленная drop-in замена CodeMirror для произвольного Markdown и монолитных документов на 200 000 слов:

- default Markdown parser/serializer нормализует синтаксис;
- basic schema не представляет front matter, raw HTML, GFM tables, task list и strikethrough;
- ProseMirror рендерит document DOM, а не виртуализированный viewport; CPU baseline уже около секунды до учёта paint;
- IME boundary выглядит корректно, но нужен ручной прогон на Windows WebView с реальными Microsoft/Google/Japanese IME.

## Путь миграции

1. Оставить Markdown и CodeMirror production source of truth.
2. Ввести версионированный block-envelope только за feature flag; сначала использовать для preview/экспериментов и не перезаписывать Markdown.
3. Добавить raw block/inline nodes и source slices для неподдержанных конструкций. Автоматическая миграция разрешается только когда analyzer подтверждает полное покрытие документа.
4. Запустить opt-in dual mode на копии документа: исходный Markdown хранится до явного подтверждения пользователя, rollback всегда открывает CodeMirror.
5. Для длинных текстов выбрать chapter/scene segmentation или доказать приемлемый layout/typing latency в Windows WebView. Не загружать 200k слов одним EditorView по умолчанию.
6. Переключать формат проекта только после fixture corpus, IME matrix и p95 latency budget; старый Markdown reader сохранять минимум одну версию формата.

## Последствия

Spike остаётся изолированным и запускаемым. Зависимости ProseMirror не попадают в production desktop bundle. Следующая работа — per-block source mapping и WebView performance/IME matrix, а не замена текущего редактора.
