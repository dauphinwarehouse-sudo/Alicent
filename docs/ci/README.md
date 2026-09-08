# Активация GitHub Actions

Конфигурация `windows-prototype.yml` подготовлена, но **не активирована**.

Подключённый GitHub token отклонил запись `.github/workflows/ci.yml` с HTTP 403 `Resource not accessible by personal access token`. Тот же набор исходников без workflow успешно записан. Права токена не изменялись, workflow обходным способом не запускался.

Владелец с правом редактирования workflows может активировать шаблон:

```powershell
New-Item -ItemType Directory -Force .github/workflows
Copy-Item docs/ci/windows-prototype.yml .github/workflows/ci.yml
git add .github/workflows/ci.yml
git commit -m "ci: enable Windows prototype checks and packaging"
git push
```

Либо обновить GitHub-подключение с правом записи **Workflows** и затем перенести шаблон. API-ключи/токены в репозиторий не добавлять.

Workflow запускает Rust checks на Linux/Windows, frontend/component/browser smoke на Linux и unsigned Windows x64 NSIS build. Installer прикрепляется к run как artifact, не публикуется в Releases. Подписания, release upload и updater нет. `cargo-metadata.json` — dependency inventory, не полноценный Rust SBOM; `npm-sbom.cdx.json` охватывает npm дерево.
