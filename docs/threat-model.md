# Модель угроз — первый срез

Активы: текст рукописи, история, целостность базы, локальные пути; в будущем API-ключи. Границы доверия: UI → native commands → repository, импортированный проект → SQLite, будущие документы → LLM → tools, LLM → внешний endpoint.

| Угроза | Текущая защита | Остаточный риск / следующая проверка |
|---|---|---|
| Path traversal в названии документа | Имена хранятся в SQLite, пути только из UUID/backend | Не предоставлять raw path инструменты |
| Symlink/junction escape | Проверка компонентов пути, БД и sidecars; Windows reparse flag | TOCTOU между проверкой и открытием не устранён; против hostile local process нужны handle-based операции |
| Обход scope чужим написанием пути | Drive-пути сравниваются без учёта регистра, segments с хвостовыми точками/пробелами и зарезервированные имена устройств (`nul`, `com1`…) отклоняются | Проверка синтаксическая: она не открывает файл, поэтому не видит junction, созданный после проверки |
| Потеря записанного текста при crash | SQLite WAL FULL; версии + журнал + индекс в одной транзакции; crash subprocess test | Не покрывает последние 650 мс черновика, аппаратные сбои диска, все виды corruption |
| Lost update | expected revision; конфликт оставляет текст в UI | Автоматического merge и multi-window UX нет |
| Повтор save/restore | UUID + hash payload + receipt в той же транзакции | Create пока не идемпотентен; не выдавать create агентам до расширения протокола |
| Повторное использование одобрения инструмента | Одобрение привязано к call ID, инструменту, его версии и каноническому хешу payload, расходуется один раз; идентификаторы непредсказуемы и не раскрывают payload hash; просроченные записи удаляются, число живых ограничено | Журнал одобрений живёт только в памяти и при переполнении теряет старые события (`dropped_journal_events`); запись в SQLite в одной транзакции с эффектом ещё не сделана |
| Недоверенная SQLite-схема | trusted_schema OFF, без extensions, parameterized SQL, quick_check | Не полный аудит вредоносного project package; до import нужен read-only quarantine/validation |
| SQL/FTS injection | bound parameters, literal quoting FTS input | Поиск ограничен 32 terms и 200 results; regex и пользовательский SQL не поддержаны |
| Утечка произведения/ключа | Ключи хранятся по opaque `secretRef`; запрос требует настроенного провайдера и явного действия; references bounded, pinned явно, excluded фильтруются до сети; telemetry нет | Реальный endpoint получает выбранный текст; будущие tools обязаны независимо проверять scope/exclusion и redaction |
| XSS из Markdown | CodeMirror показывает текст, diff рендерится React text nodes, не innerHTML | Rich text preview/HTML import потребует sanitization и protocol allowlist |
| Prompt injection | Reference documents помечены как read-only context; текущий editor flow не запускает tools; провенанс промпта решает политика инструмента — смешанный промпт допускают только read-only политики с явным opt-in, любая запись требует промпта целиком от пользователя, а injection-сигналы запрещают вызов | Рантайм доверяет переданной ему классификации провенанса: сам детектор сигналов и never-promote-document-to-system ещё не реализованы, exclusion нужно перепроверять после чтения |
| Незаметный rollback | UI сравнение и явная кнопка; новая версия, старое содержимое сохранено | Native restore пока user-only command; tool runtime обязан независимо проверять approval |
| Зависание от больших данных | Пагинация, один текст в UI, CodeMirror viewport, native IO вне UI thread | Полная сериализация текста и snapshots; нет performance acceptance для 5M слов |

## Безопасные настройки

Нет remote origins, shell/FS/HTTP plugins, автообновления или фоновой телеметрии. Rust commands доступны главному локальному окну. Native chooser не принимает путь из модели. Полные ошибки SQLite и системные пути не возвращаются в UI; типовые сообщения русские. Секреты не принимаются ни одним действующим интерфейсом.

## До публичного релиза

- Handle-based защита от path races, integrity/foreign key/version log validation.
- Persist approval и execution журналов в SQLite в одной транзакции с эффектом: сейчас они только в памяти и теряются при переполнении и при выходе.
- Отдельный runtime authorization и user approval token для tool execution; per-document exclusion уже имеет repository/UI regression tests.
- Windows Credential Manager adapter; redaction-тесты и запрет ключей в конфиге.
- Подписанный installer/updater, SBOM всего Rust+JS дерева, pin actions по SHA и аудит supply chain.
- Atomic backup/restore, migration rollback, power-loss и native Windows E2E на Win10/11.
