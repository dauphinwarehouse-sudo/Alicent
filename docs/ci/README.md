# Статус GitHub Actions

Workflow активирован: `.github/workflows/ci.yml` добавлен в ветку `feat/local-core-foundation` после обновления разрешения Workflows. Прежний блокер записи снят. Ручное копирование шаблона больше не требуется.

Чтение check runs через текущее GitHub-подключение возвращает HTTP 403. Это не означает, что сборка упала или прошла: результат пока не подтверждён. Полезные разрешения для работы с runs/logs — Actions read/write; доступ к Checks зависит также от типа токена/подключения. Не нужно выдавать Administration ради чтения результатов.

Workflow запускает Rust checks на Linux/Windows, frontend/component/browser smoke на Linux и unsigned Windows x64 NSIS build. Installer после успешной сборки прикрепляется к run как artifact, не публикуется в Releases. Подписания и updater нет. `cargo-metadata.json` — dependency inventory, не полноценный Rust SBOM; `npm-sbom.cdx.json` охватывает npm дерево.

`windows-prototype.yml` в этом каталоге сохранён как исходный шаблон; действующий workflow находится в `.github/workflows/ci.yml`.
