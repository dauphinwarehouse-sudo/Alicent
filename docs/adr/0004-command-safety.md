# ADR 0004 — Revision checks и идемпотентный command ledger

Статус: принято для save/restore, расширение на все agent commands обязательно.

Каждая запись проверяет expected revision внутри IMMEDIATE transaction. Это предотвращает последнее-записавшее-окно-побеждает. UUID команды связан с SHA-256 параметров; повтор возвращает исходный результат, другой payload с тем же ID отвергается. История append-only на уровне публичного API. Восстановление — новая версия, не UPDATE старых versions.

Сейчас actor = `user:local`, UI user-only. Перед агентами actor/task metadata должно приходить из доверенного runtime, не из аргументов модели. Approval bind к payload hash и точному scope, проверка в runtime до repository. Публичного raw SQL/FS tool не будет.

Недостатки: receipts хранят полный документ и занимают место, create ещё не идемпотентен. Это блокеры для agent write tools и больших проектов, а не допустимые незадокументированные ограничения.
