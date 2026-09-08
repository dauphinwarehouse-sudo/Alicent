# Контракты AI provider, tool и agent runtime

**Статус: интерфейсы, не действующий AI.** Код: `packages/contracts/src/index.ts`. В текущем приложении нет API-ключей, сетевых AI-запросов или запуска инструментов агентами.

## ProviderAdapter

`capabilities(config, signal)` и `stream(config, messages, tools, signal)`. Конфигурация хранит opaque `secretRef`, а не ключ. Протоколы: `openai-chat`, `openai-responses`, `anthropic-messages`. Сообщения сохраняют tool call IDs и связь tool result с вызовом, не схлопывают всё в пользовательский текст.

Нормализованные события: `text_delta`, явно возвращённый провайдером `reasoning_summary`, завершённый `tool_call`, `usage`, `done`, `error`. Частичные аргументы инструмента не выдаются исполнителю до полного JSON parse + schema validation. AbortSignal должен закрывать stream и request. Error code не должен содержать headers, body или ключи.

Реализация phase 2 обязана добавить handshake/capability detection, ограниченные retries, SSE parser с разрывами UTF-8/chunk boundaries, нормализацию ошибок, token usage, redaction и contract fixtures всех трёх протоколов. Нельзя ретраить уже исполненные tool side effects без durable idempotency.

## Tool

Формальный registry entry: имя, описание, JSON input/output schema, risk, confirmation, timeout, scope. `ToolContext` несёт проект, задачу, actor, command ID, разрешённые document IDs и cancellation.

Перед execute runtime обязан:
1. Проверить JSON Schema и лимиты.
2. Проверить tool allowlist и текущий проект.
3. Проверить каждый запрашиваемый document ID по scope и AI-exclusion, в том числе внутри результатов поиска.
4. Для destructive/external всегда запросить подтверждение; explicit confirmation имеет приоритет над auto-reversible policy.
5. Привязать одобрение к задаче, конкретной команде и хешу payload; нельзя использовать общий boolean от модели.
6. Для записи использовать только command layer с checkpoint/diff и атомарной транзакцией, никогда raw FS.
7. Проверить output schema, лимит результата и redaction до возврата модели.

Чистая функция `needsApproval` реализует лишь выбор необходимости подтверждения. Она не заменяет authorization и не является готовым security runtime. Create/move/delete массовые инструменты не зарегистрированы.

## AgentRuntime

`enqueue`, `cancel`, `resume`; durable task содержит budget (steps/deadline/cost) и lifecycle. Domain валидирует допустимые переходы и запрещает незаметный restart terminal-задач. Планируемое исполнение: очередь → планирование → подтверждение при необходимости → шаги → завершение. Pause/resume должны восстанавливать лишь подтверждённый tool ledger, не повторять side effects.

До реализации необходимы отдельные контексты задач, персистентный step log, лимиты стоимости/времени, scopes и тесты двух конкурирующих задач. Интерфейсы не означают, что эти гарантии уже доступны.
