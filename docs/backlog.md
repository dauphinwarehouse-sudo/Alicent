# План реализации и зависимости

Исходное ТЗ сохранено без изменений. Оборванный конец не восстановлен догадками. Лицензия MIT уже существовала в репозитории и оставлена. Minimum hardware и сертификат подписи пока не определены; это не блокирует локальный каркас.

## Первый вертикальный срез

| ID | Результат | Зависит от | Статус / критерий |
|---|---|---|---|
| F01 | Monorepo, Domain, UI/IPC контракты | — | Реализовано; Rust и TS compile/tests |
| F02 | SQLite v1, история, FTS, revision guards | F01 | Реализовано; reopen/conflict/crash/property tests |
| F03 | Windows shell, native picker | F02 | Код и шаблон CI добавлены; native запуск ещё не подтверждён |
| F04 | Markdown editor, автосохранение, diff/restore | F01,F02,F03 | Реализовано; component/browser smoke отдельно от native |
| F05 | Architecture, ADR, threat model | F01–F04 | Документация в этой ветке |
| F06 | Windows installer | F03,F04 | CI-сборка installer подтверждена; нужен smoke установленного приложения на Win10/11 |

**Phase 0 не завершена:** нет испытаний больших документов, реальных OpenAI/Anthropic endpoints и rich-text прототипа. Phase 1 также неполна: checkpoints и recovery реализованы, но нет полного набора project operations. Наличие каркаса не равно прохождению exit criteria.

## Следующие независимые PR

| ID | Объём | Зависимости | Проверка приёмки |
|---|---|---|---|
| P0.1 | Benchmark datasets 10k/500k/5M слов; 20k узлов; 200k-word документ | F02,F04 | В работе: storage harness добавлен; остаются Windows app startup/input/memory и отчёт на объявленном ПК |
| P0.2 | TipTap/ProseMirror rich-text spike, lossless Markdown/block schema | P0.1 | Undo/IME/кириллица; latency target, решение ADR |
| P1.1 | Rename/move/archive/duplicate; order keys; идемпотентный create | F02 | В работе: rename/duplicate; остаются move/archive/order keys/idempotent create |
| P1.2 | Checkpoints + multi-document transaction + diff | P1.1 | Полный rollback набора, conflict без частичного применения |
| P1.3 | Backup API, recovery UI, migration harness | F02 | Crash на каждом шаге, restore копии и foreign key validation |
| P2.1 | Credential Manager, endpoint settings, privacy controls | F03, threat model | Ключи отсутствуют в БД/логах; safe TLS; localhost explicitly configured |
| P2.2 | OpenAI Chat/Responses и Anthropic Messages adapters | P2.1 | SSE contract tests, fragmented UTF-8, timeout/abort, real endpoint smoke |
| P3.1 | Tool registry, JSON schemas, scopes и approvals | P1.2,P2.2 | Path/scope/prompt injection denial; approval payload binding |
| P3.2 | Durable task queue, budgets, profiles, custom agent UI | P3.1 | Cancel/resume/restart, два конкурирующих агента без lost writes |
| P4.1 | Entities/relations/timeline и excluded-context rules | P1.1 | Scope tests, backlinks, lazy tree |
| P4.2 | Stable chunks, incremental index, summaries и optional vectors | P4.1,P2.2,P0.1 | Изменяется только затронутый индекс; bounded token budget |
| P5.1 | Markdown/TXT/DOCX/EPUB import/export, затем PDF/Fountain | P1.3,P0.2 | Unicode round-trip, bounded memory, zip-bomb/path validation |
| P5.2 | Native E2E, diagnostics preview, accessibility и release audit | P3.2,P4.2,P5.1 | Все mandatory scenarios, нулевые critical findings |
| P5.3 | Подпись Windows installer, update signing, полный SBOM | P5.2 + сертификат владельца | Чистая Win10/11 установка/обновление/откат |
| P6.1 | Actor metadata, operation protocol v2, sync transport ADR | P3.2 | Документированные optimistic-concurrency hooks, без сетевой коллаборации |

## Ближайший шаг

Native Windows artifact и P1.3 подтверждены в CI. Завершить P0.1 на объявленном Windows-ПК, затем P0.2. Сначала измеримые границы редактора; не подменять агентный runtime кнопками-заглушками.
