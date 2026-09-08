# Проверки первого среза

Дата запуска: 2026-09-08 UTC. Среда локального исполнения: Linux x64 sandbox, Node 24.14.1, Rust 1.94.0, системный Chromium. Это **не** проверка установки на Windows.

## Выполнено локально

| Команда / область | Результат |
|---|---|
| `cargo test --locked` | 15 Rust tests passed: 2 domain + 13 repository, включая subprocess crash и property-based сценарий (32 случая) |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed для default workspace members: domain/repository и их tests |
| `cargo fmt --all --check` | Passed |
| `npm run lint` | Passed |
| `npm run typecheck` | Passed |
| `npm test` | 12 tests passed: serialized autosave, lost acknowledgement retry, confirmation policy, React components |
| `npm run build` | Production frontend собран; тестовый IPC-double не является entry point |
| `CHROMIUM_PATH=… npm run test:ui` | 4 browser smoke tests passed: preview, edit/autosave/diff/restore, new scene/folder/search, mobile overflow |
| `npm install` audit | 0 vulnerabilities reported при установке; не заменяет аудит всего Rust/JS supply chain |

## Что проверялось в ядре

Повторное открытие с кириллицей/Unicode, FTS обновление после restore, stale-revision conflict без лишней версии, привязка idempotency к payload, запрет scene-parent и чужих UUID, неизвестный формат без downgrade, неподдерживаемый размер документа, literal FTS input, отсутствие создания базы при неверном open, symlink/sidecar escape на Unix, сохранность commit после аварийного выхода отдельного процесса с незавершённой транзакцией.

UI-тесты используют явный тестовый `ProjectPort`, а не реальный Tauri/SQLite. Они подтверждают взаимодействие компонентов, **не native IPC или Windows filesystem**. Искусственные тексты находятся только в test fixture. Юнит-тесты CodeMirror-порта не заменяют performance benchmark.

## Не подтверждено

- Native Rust/Tauri сборка и установленное приложение Windows 10/11. Workflow настроен; его результат нужно проверять отдельно. Установщик здесь не заявлен готовым.
- Подпись installer/updater; сертификат не предоставлен, релиз не опубликован.
- Весь mandatory тест-план исходного ТЗ, native E2E, accessibility audit, power-loss на физическом диске.
- 5 млн слов, 20k документов, 200k-word input latency, startup/open/search p95 и память.
- OpenAI Chat/Responses и Anthropic endpoint tests: adapters пока отсутствуют.
- Migration tests: неизвестные схемы отвергаются, миграции пока отсутствуют.

## Вывод

Проверенный локальный каркас и первый вертикальный срез готовы к review. **Phase 0/Phase 1 и MVP не завершены.** До работы с единственным экземпляром важной рукописи нужны native validation и резервное копирование.
