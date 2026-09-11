# Alicent

Настольное приложение под Windows для работы над длинной прозой: проект целиком лежит локально в SQLite, есть версии и чекпоинты, поиск по тексту, резервные копии и подключаемые AI-провайдеры, которые не могут менять текст без явного одобрения.

## Состояние

Прототип, не релиз. Установщик собирается без подписи, нативная валидация на Windows и полный MVP не завершены. Актуальный список того, что сделано и что нет, — в [CHANGELOG.md](CHANGELOG.md) и [docs/backlog.md](docs/backlog.md). Не держите в нём единственную копию рукописи.

## Как устроен репозиторий

- `apps/desktop` — приложение: фронтенд на React и Tauri-крейт `alicent-desktop` в `src-tauri`
- `crates/domain` — типы предметной области и валидация переходов состояний
- `crates/project-repository` — проект в SQLite: документы, версии, чекпоинты, поиск, бэкап и восстановление
- `crates/tool-runtime` — граница безопасности для инструментов: разрешения, одобрения, журнал, бюджеты
- `crates/provider-wire` — работа с AI-провайдерами и секреты через Windows Credential Manager
- `crates/document-io` — импорт и экспорт документов
- `crates/story-graph` — граф сущностей произведения
- `docs` — архитектура, формат проекта по версиям, модель угроз, контракты рантайма, ADR
- `scripts` — `report-ci.cjs` для отчёта CI и `native-windows-e2e.ps1` для проверки установленного приложения

## Требования

- Windows 11 или Windows Server 2022 — CI собирает на `windows-2022`
- Rust 1.94.0, версия закреплена в `rust-toolchain.toml`
- Node.js 22

## Проверки

Те же команды, что выполняет CI:

```powershell
npm ci
npm run check
npx playwright install chromium
npm run test:ui

cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## Запуск и сборка

```powershell
npm run tauri -w @alicent/desktop -- dev

cargo check --locked -p alicent-desktop
npm run tauri -w @alicent/desktop -- build -- --locked
```

Неподписанный установщик появляется в `target/release/bundle/nsis`.

Важная оговорка про clippy: `default-members` в корневом `Cargo.toml` не включает `apps/desktop/src-tauri`, поэтому `cargo clippy --all-targets` не смотрит на крейт приложения. Компиляцию его закрывает `cargo check -p alicent-desktop`, формат — `cargo fmt --all`.

## Безопасность

Модель угроз и остаточные риски — [docs/threat-model.md](docs/threat-model.md). Правила для инструментов и одобрений — [docs/runtime-contracts.md](docs/runtime-contracts.md). Как сообщить об уязвимости — [SECURITY.md](SECURITY.md).

## Участие

[CONTRIBUTING.md](CONTRIBUTING.md).

## Лицензия

MIT, см. [LICENSE](LICENSE).
