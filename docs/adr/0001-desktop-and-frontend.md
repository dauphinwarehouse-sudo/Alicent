# ADR 0001 — Tauri 2 + React/TypeScript, изолированное Rust-ядро

Статус: принято для прототипа, измерение startup/memory ожидается.

Контекст: нужен Windows desktop с локальной БД и будущими заменяемыми UI/transport. Выбираем рекомендованный Tauri 2 и React, Rust workspace с отдельными domain и repository. React отвечает только за presentation, Rust — за IO и команды. На старте используем локальное React state и EditorSession вместо добавления Zustand/TanStack Query до появления потребности.

Альтернативы: Electron проще для единого JS backend, но дублирует browser runtime; .NET удобен на Windows, но связывает UI с отдельной экосистемой. Решение не утверждает выигрыш в цифрах до замеров.

Последствия: WebView2/Windows build toolchain обязательны; Linux sandbox может проверить ядро/веб, но не Windows installer. CI собирает Windows отдельно. Никаких скрытых облачных сервисов.
