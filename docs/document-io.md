# Document I/O foundation

`crates/document-io` — независимая граница безопасного ввода/вывода документов.
Она не подключена к UI, project repository/schema, editor, provider,
archive/move, story graph или tool runtime.

## Контракт первой очереди

`import_text` и `export_text` принимают обычные `Read`/`Write`, работают
фиксированным буфером и возвращают счётчики фактически прочитанных/записанных
байтов. Markdown и TXT обязаны быть корректным UTF-8. Байты корректного UTF-8
сохраняются без изменения, включая BOM, CRLF/LF и composed/decomposed Unicode.

Лимиты задаются вызывающей стороной через `IoLimits`; недопустимый нулевой или
слишком большой буфер отклоняется до чтения.

## Граница ZIP-контейнеров

`ArchiveBudget` предназначен для будущих DOCX/EPUB adapters. Parser обязан
передать каждую запись в один общий budget и копировать распакованные данные
через `copy_entry`. Проверяются и объявленные metadata, и фактический поток:

- число записей;
- размер одной и всех распакованных записей;
- compression ratio;
- совпадение declared/actual expanded size;
- длина и глубина member path;
- отсутствие absolute/traversal, backslash, drive prefix и NTFS ADS paths.

Нельзя сначала распаковывать запись в память, а затем вызывать guard.
Filesystem extraction также не входит в контракт: adapters должны читать только
необходимые members и не создавать пути из недоверенных имён.

## Текущий статус

| Формат | Импорт | Экспорт |
|---|---:|---:|
| Markdown (UTF-8) | да | да |
| TXT (UTF-8) | да | да |
| DOCX | нет, только security foundation | нет |
| EPUB | нет, только security foundation | нет |
| PDF | нет | нет |
| Fountain | нет | нет |

DOCX/EPUB можно добавлять отдельным изменением только поверх общего budget и
с отдельными container/XML fixtures. PDF/Fountain не считаются готовыми.