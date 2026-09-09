# ADR 0009 — Изолированный story graph для P4.1

Статус: экспериментальный domain slice; принят для проверки модели, не для production persistence.

## Контекст

P4.1 требует сущности персонажей и мест, отношения, timeline, backlinks, lazy tree и правила excluded context. Параллельно развиваются archive schema v3 и document operations. Подключение новой модели к `project.sqlite3`, desktop IPC или UI в этом срезе создало бы конфликт схем и преждевременно закрепило формат хранения.

## Решение

Добавлен самостоятельный Rust crate `alicent-story-graph` в `crates/story-graph`. Он не зависит от `alicent-domain`, `alicent-project-repository`, Tauri, provider/import/tool-runtime и не открывает файлов. Состояние живёт только в памяти процесса. Production schema, миграции и UI не изменяются.

### Модель

- `Entity` имеет глобально уникальный `EntityId`, тип `Character` или `Place`, каноническое имя, уникальный набор aliases, необязательного родителя того же типа и `ContextRule`.
- `Relation` связывает две разные существующие сущности, имеет валидированный тип, directed/undirected семантику, необязательный диапазон timeline и `ContextRule`. Для undirected relation endpoints канонически упорядочиваются.
- `Event` имеет непустой заголовок, валидный `TimelineRange`, хотя бы одну ссылку, набор персонажей-participants, набор мест и `ContextRule`.
- `StoryInstant(i64)` — намеренно нейтральная порядковая координата. Прототип не навязывает календарь, эру или точность. Значение даты и преобразование календарей остаются обязанностью будущего application/storage adapter.

### Строгие invariants

Доменные поля закрыты, значения строятся валидирующими constructors. Прямой `Deserialize` намеренно не реализован: будущий storage DTO обязан пройти constructors и graph insert methods.

`StoryGraph` отклоняет:

1. UUID, повторно использованный любой сущностью, отношением или событием;
2. dangling references;
3. self-relations;
4. participant не типа `Character` и location не типа `Place`;
5. пустое событие без participants и locations;
6. timeline с `end < start`;
7. parent другого типа и reparent, создающий цикл;
8. пустые/control/слишком длинные имена, aliases, relation kinds, context/query identifiers;
9. context, одновременно присутствующий в include и exclude.

Мутация выполняется только после полной проверки, поэтому ошибка не оставляет частично обновлённые индексы.

### Scope и excluded context

`ContextRule.include` пустой — объект доступен во всех scopes, кроме явно исключённых. Непустой include требует пересечения с active contexts запроса. Любое пересечение с exclude всегда побеждает include.

`GraphView` — единая scoped projection. Она не возвращает excluded entity. Relation или event также скрывается, если скрыта хотя бы одна referenced entity; тем самым query, timeline и backlinks не раскрывают ID или факт связи с исключённым контекстом. Это fail-closed решение. Если продукту позже понадобится redacted edge, он должен быть отдельным DTO/use case, а не ослаблением domain projection.

### Индексы и lazy API

`StoryGraph` поддерживает детерминированные `BTree*` индексы:

- parent → children для `roots()`/`children()`;
- entity → typed `Backlink`;
- `(StoryInstant, EventId)` для timeline ordering;
- primary maps для entity/relation/event lookup.

`GraphView::{roots, children, query, relations, timeline, backlinks}` возвращают iterators и фильтруют по scope по мере чтения. API не строит рекурсивное дерево и не материализует полную scoped копию. Потребитель сам задаёт глубину обхода и прекращает чтение. BTree ordering обеспечивает воспроизводимые тесты и стабильную пагинацию в будущем, но cursor/page DTO пока не закреплён.

## Проверки

Contract tests покрывают:

- dangling/wrong-kind links, cycles, cross-kind parents, self-relations, invalid ranges и UUID collisions;
- include/exclude precedence и multi-context scope;
- отсутствие утечки excluded entity через relations, timeline и backlinks;
- typed backlinks для обоих концов relation и event roles;
- deterministic lazy roots/children/text query и ordered/windowed timeline.

Crate включён в workspace/default members, поэтому выполняется обычными `cargo fmt --all --check`, `cargo clippy --locked --all-targets -- -D warnings` и `cargo test --locked` на Linux и Windows. В workflow добавлен только push trigger `spike/**`, чтобы требуемая ветка получила тот же CI; состав jobs не менялся.

## Последствия и ограничения

- Это prototype API, а не archive schema v3 и не обещание on-disk compatibility.
- Нет CRUD/undo, document links, FTS/vector index, summaries, IPC, UI, import/export, permissions или agent tools.
- In-memory indexes пересобираются будущим adapter; текущий crate не сериализует `StoryGraph` целиком.
- Query text — простой Unicode lowercase substring. Locale-specific normalization и полнотекстовый индекс относятся к P4.2.
- Event visibility fail-closed может скрывать событие при одном excluded participant; продуктовый UX redaction требует отдельного решения.

## План будущей интеграции

1. После стабилизации archive schema v3 определить отдельные storage DTO/tables и migration ADR. Не добавлять поля story graph в существующие document rows.
2. Реализовать repository adapter в отдельном crate: read DTO → validating constructors → atomic `StoryGraph`; write через собственные transactions и revision guards.
3. Добавить fixture/migration tests: malformed rows, dangling refs, cycles, rollback, reopen и deterministic index rebuild. Миграция обязана создавать backup по правилам project repository.
4. Согласовать context identifiers с document-level AI exclusion/scopes. До этого не связывать `ContextRule` с tool approvals или provider payloads.
5. Добавить application use cases с bounded page/cursor API поверх lazy iterators. Только после contract tests открыть IPC, затем отдельный UI PR.
6. P4.2 строит stable chunks/index/summaries поверх публичной scoped projection, не обходя excluded-context фильтр.
7. Перед production adoption зафиксировать calendar semantics, deletion policy, relation cardinality/uniqueness и redaction UX отдельными ADR.

## Альтернативы

- Расширить `alicent-domain`: отклонено — смешивает текущие document/task contracts с экспериментальной моделью и повышает риск параллельных конфликтов.
- Сразу мигрировать SQLite: отклонено — archive schema v3 и document operations ещё меняются, формат P4.1 не доказан.
- Возвращать materialized recursive tree: отклонено — неограниченная глубина и стоимость, неудобная пагинация, риск случайного обхода excluded nodes.
