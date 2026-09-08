# ADR 0008 — Безопасная нормализация OpenAI Chat stream

Статус: инфраструктурный срез P2.2; не подключение модели к приложению.

## Решение

Добавлен `ChatDecoder` в `crates/provider-wire`. Это чистый Rust-разбор уже полученных байтов: без HTTP, файлов, API-ключей, платных запросов и выполнения инструментов. Поддерживается один choice (`index: 0`) OpenAI Chat Completions, текст и client-function tools. OpenAI Responses и Anthropic пока имеют только request builders и общие SSE/tool primitives из ADR 0007 — их vendor-normalizers не реализованы.

### Контракт жизненного цикла

1. `ChatDecoder::new(allowed_names)` проверяет ограниченный список имён инструментов.
2. `push(bytes)` принимает порции в пределах лимитов `SseDecoder`. Возвращает только `ChatEvent::TextDelta`: это предварительный текст для отображения, не свидетельство успешного завершения.
3. `finish()` вызывается после окончания чтения транспорта. Успех требует согласованного `finish_reason` (`stop` без tools либо `tool_calls` с tools), маркера `[DONE]` и чистой границы SSE. EOF без этих условий не является успехом. После ошибки вызывающая сторона обязана отбросить незавершённый ответ.
4. Только успешный `finish()` возвращает `ChatCompletion` с целой пачкой предложений инструментов. Никакие предложения не выходят из `push`, даже после получения полностью корректного JSON. Обрыв, `length`, `content_filter`, refusal, отмена или повреждённый вызов не выпускают пачку.
5. Ошибка, отмена и завершение необратимо закрывают decoder. Повторное использование возвращает `Closed`. Транспорт в будущем обязан отдельно прерывать HTTP-запрос и применять timeout/cost/token budgets.

## Проверки входных данных

- Постоянные response ID и model на всём потоке; строгий `chat.completion.chunk`, один choice и роль assistant.
- Перемежающиеся tool calls собираются по индексу; повторный call ID, изменение ID/имени, неизвестное имя и неоднозначный индекс в одном delta отклоняются. Начальный delta должен содержать полные ID, имя и `type: function`; фрагментация аргументов поддерживается, фрагментация метаданных — нет.
- Usage хранит только предоставленные провайдером неотрицательные целые input/output/total tokens; проверяются сумма и переполнение. Отсутствующий usage остаётся `None`, не превращается в нулевую стоимость. Usage принимается один раз после успешного finish_reason, включая тот же final-choice chunk.
- `reasoning_content`, audio, legacy function calls и другие неизвестные непустые delta-поля отвергаются как неподдерживаемые, а не молча теряются. Reasoning/signature state и multimodal output не реализованы.
- JSON объектов провайдера и аргументов теперь отклоняет повторные ключи на любой глубине, в том числе одинаковые после декодирования Unicode escape. Прежний общий `ToolArguments` принимал JSON через `serde_json::Value`, где последний повторный ключ незаметно заменял первый. Сохранены recursion limit, отказ от trailing data, лимиты байтов и требование object для аргументов.
- Публичные ошибки содержат только фиксированные коды/описания. Debug у новых событий и результата показывает метаданные, а не текст, аргументы или vendor error body. Не следует логировать содержимое `ToolProposal` напрямую.

## Границы безопасности и проверки

Имя в allowlist и корректный JSON **не являются разрешением на выполнение**. JSON Schema validation, current project, document scopes, AI-exclusions, payload-bound approvals, idempotent command layer и отмена транспорта по-прежнему обязательны до появления runtime. В UI нет работающего AI-чата.

Добавлены синтетические Rust contract/regression tests: Unicode на каждой границе transport chunk; параллельные вызовы; каждый усечённый префикс tool-ответа; неверные terminal markers, stop reasons и usage; подмена метаданных; duplicate JSON keys; redaction; отмена; лимиты. Тесты подключены обычным `cargo test --locked`, crate уже входит в default workspace members.

Локальное окружение этой работы не содержит Rust и не имеет прямого доступа к GitHub для clone. Поэтому Rust/Clippy/rustfmt и Windows packaging проверяются в существующем GitHub Actions CI; итог конкретного commit нужно смотреть в его `alicent/ci` и PR-отчёте, а не считать этот документ доказательством прохождения тестов. Native установленный Windows E2E, live endpoint tests и производительность больших рукописей этим срезом не подтверждаются.

Следующий шаг: отдельные normalizers Responses/Anthropic; затем native vault/transport с явными privacy controls и opt-in live endpoint smoke. Сохранность данных и native Windows validation остаются отдельными обязательными этапами.
