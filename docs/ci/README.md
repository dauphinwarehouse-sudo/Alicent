# Статус GitHub Actions

Workflow активирован: `.github/workflows/ci.yml` добавлен в ветку `feat/local-core-foundation` после обновления разрешения Workflows. Прежний блокер записи снят. Ручное копирование шаблона больше не требуется.

Чтение check runs через текущее GitHub-подключение возвращает HTTP 403. Это не означает, что сборка упала или прошла: результат пока не подтверждён. Полезные разрешения для работы с runs/logs — Actions read/write; доступ к Checks зависит также от типа токена/подключения. Не нужно выдавать Administration ради чтения результатов.

Workflow запускает Rust checks на Linux/Windows, frontend/component/browser smoke на Linux и unsigned Windows x64 NSIS build. Installer после успешной сборки прикрепляется к run как artifact, не публикуется в Releases. Подписания и updater нет. `cargo-metadata.json` — dependency inventory, не полноценный Rust SBOM; `npm-sbom.cdx.json` охватывает npm дерево.

`windows-prototype.yml` в этом каталоге сохранён как исходный шаблон; действующий workflow находится в `.github/workflows/ci.yml`.

## Автоматическая обратная связь

После обновления прав Commit statuses доступен. Workflow настроен публиковать `alicent/ci`: pending при запуске и success/failure после всех обязательных jobs. Отдельная report job использует ограниченный GITHUB_TOKEN своего репозитория; personal token в runner не передаётся.

В открытый PR этой же ветки публикуются итоги jobs, ссылка на run и ограниченные error annotations GitHub. Сырые логи не копируются. Отчёт запускается только по trusted push/manual событиям, не на `pull_request_target`. Пропущенная или отменённая обязательная job не считается успехом. Скрипт отчёта покрыт тремя отдельными Node tests.

До получения статуса success Windows installer по-прежнему не считается собранным. Обязательные review/branch rules этим механизмом не изменяются.
