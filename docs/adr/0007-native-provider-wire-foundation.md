# ADR 0007 — Нативные основы AI wire-протоколов

Статус: принято, инфраструктурный срез; не действующий AI-чат.

## Граница модуля

`crates/provider-wire` — чистая Rust-библиотека без Tauri, SQLite, HTTP, файлового доступа и API-ключей. Она формирует тела запросов и предоставляет строительные блоки потока. Включена в default-members Cargo, поэтому проверяется обычными core jobs Linux/Windows. UI не подключён к библиотеке, скрытых запросов нет.

Реализовано:

- Текстовые запросы и client-function tools для OpenAI Chat Completions, OpenAI Responses и Anthropic Messages. Системная роль не смешивается с текстом документов; tool-call ID связывается с ответом инструмента. Anthropic system вынесен отдельно, параллельные tool_result объединяются в следующий user turn.
- Предварительная проверка транскрипта: запрещены неизвестные, повторные и неотвеченные call ID, пользовательский ход между вызовом и его ответом, позднее добавление system-сообщения. Имя инструмента должно присутствовать в переданных определениях. Это проверка формы, **не проверка прав на документы**.
- SSE framing с сохранением UTF-8 на произвольных границах чанков, BOM, CR/LF/CRLF, многострочных `data`, комментариев. EOF не считается подтверждённым завершением. Ошибка и отмена закрывают decoder; `id`/`retry` не вызывают автоматическое переподключение.
- Сборщик аргументов нескольких перемежающихся tool calls. До `finish` не возвращает proposals; повреждённый JSON, scalar/array вместо object или неизвестное имя отклоняет всю пачку. Возвращается **неавторизованное предложение**, никогда готовая команда. JSON Schema validation, permissions, approval и execution отсутствуют и обязательны в будущем runtime.
- Публичная allowlist HTTP-классификация без response body/headers; retry-policy допускает до двух повторов только для 429/502/503/504 и только до начала ответа/side effect. Реальных повторов библиотека не выполняет.
- Лимиты: request 8 МиБ (подсчёт без предварительного копирования всего тела), SSE chunk/line 1 МиБ, event 2 МиБ, stream 32 МиБ; до 64 tool calls, аргументы 1 МиБ на call и 4 МиБ всего. Нужны также будущие time/token/cost budgets.

OpenAI-запросы явно устанавливают `store:false`; это **не обещание нулевого хранения у провайдера** и не заменяет его политику данных. Совместимость конкретных OpenAI-compatible endpoints и моделей ещё не проверена. `max_completion_tokens` — текущий OpenAI Chat baseline, не универсальное поле всех сторонних серверов.

## Ещё не реализовано

HTTP transport, URL allowlist/redirect policy, Windows Credential Manager, proxy/TLS настройки, capability detection, нормализация vendor stream events в `ModelEvent`, точный usage, reasoning/vision/server tools, возобновление, UI выбора провайдера, tool registry/JSON Schema validation, AI-exclusions, durable agent queue и подтверждение правок. Модельный tool call не может исполнить код в текущем приложении.

Особенно важно: reasoning-модели могут требовать сохранения opaque reasoning/signature items между turns. Библиотека этого среза не имеет таких типов, поэтому не должна заявляться совместимой с такими режимами или молча удалять эти items в будущем адаптере.

## Проверки и источники форматов

Контрактные Rust fixtures — локальные синтетические примеры, не записанные production-ответы. Проверены три request shape, call/result linkage, порядок ролей, ошибки, каждый размер чанка Unicode SSE, truncation, лимиты, interleaved tool arguments, отмена и правила retry. API-ключи не использовались, платные вызовы не выполнялись. Следующий срез должен добавить vendor-normalizers и native transport/vault, затем live opt-in tests.

- [OpenAI Chat API](https://platform.openai.com/docs/api-reference/chat/create): роли assistant/tool, JSON arguments и `tool_call_id`, usage chunk перед `[DONE]`.
- [OpenAI Responses API](https://platform.openai.com/docs/api-reference/responses/create): `function_call`, `function_call_output`, `call_id`, `store`, `strict`.
- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages): system и блоки `tool_use`/`tool_result`.
