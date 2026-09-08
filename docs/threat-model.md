# Модель угроз — первый срез

Активы: текст рукописи, история, целостность базы, локальные пути; в будущем API-ключи. Границы доверия: UI → native commands → repository, импортированный проект → SQLite, будущие документы → LLM → tools, LLM → внешний endpoint.

| Угроза | Текущая защита | Остаточный риск / следующая проверка |
|---|---|---|
| Path traversal в названии документа | Имена хранятся в SQLite, пути только из UUID/backend | Не предоставлять raw path инструменты |
| Symlink/junction escape | Проверка компонентов пути, БД и sidecars; Windows reparse flag | TOCTOU между проверкой и открытием не устранён; против hostile local process нужны handle-based операции |
| Потеря записанного текста при crash | SQLite WAL FULL; версии + журнал + индекс в одной транзакции; crash subprocess test | Не покрывает последние 650 мс черновика, аппаратные сбои диска, все виды corruption |
| Lost update | expected revision; конфликт оставляет текст в UI | Автоматического merge и multi-window UX нет |
| Повтор save/restore | UUID + hash payload + receipt в той же транзакции | Create пока не идемпотентен; не выдавать create агентам до расширения протокола |
| Недоверенная SQLite-схема | trusted_schema OFF, без extensions, parameterized SQL, quick_check | Не полный аудит вредоносного project package; до import нужен read-only quarantine/validation |
| SQL/FTS injection | bound parameters, literal quoting FTS input | Поиск ограничен 32 terms и 200 results; regex и пользовательский SQL не поддержаны |
| Утечка произведения/ключа | Нет сетевого транспорта и ключей; UI CSP без произвольного HTTP, нет telemetry | Будущие endpoint запросы требуют явного провайдера и scoped context |
| XSS из Markdown | CodeMirror показывает текст, diff рендерится React text nodes, не innerHTML | Rich text preview/HTML import потребует sanitization и protocol allowlist |
| Prompt injection | Нет агентного исполнителя | До AI: пометка недоверенных фрагментов, scopes, approvals, never-promote-document-to-system |
| Незаметный rollback | UI сравнение и явная кнопка; новая версия, старое содержимое сохранено | Native restore пока user-only command; tool runtime обязан независимо проверять approval |
| Зависание от больших данных | Пагинация, один текст в UI, CodeMirror viewport, native IO вне UI thread | Полная сериализация текста и snapshots; нет performance acceptance для 5M слов |

## Безопасные настройки

Нет remote origins, shell/FS/HTTP plugins, автообновления или фоновой телеметрии. Rust commands доступны главному локальному окну. Native chooser не принимает путь из модели. Полные ошибки SQLite и системные пути не возвращаются в UI; типовые сообщения русские. Секреты не принимаются ни одним действующим интерфейсом.

## До публичного релиза

- Handle-based защита от path races, integrity/foreign key/version log validation.
- Отдельный runtime authorization, user approval token, excluded-context regression tests.
- Windows Credential Manager adapter; redaction-тесты и запрет ключей в конфиге.
- Подписанный installer/updater, SBOM всего Rust+JS дерева, pin actions по SHA и аудит supply chain.
- Atomic backup/restore, migration rollback, power-loss и native Windows E2E на Win10/11.
