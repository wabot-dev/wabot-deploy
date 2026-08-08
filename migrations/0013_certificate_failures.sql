-- A failure belongs to the name that failed.
--
-- Until now there was one `acme_error` for the whole node. With one
-- certifiable name that was the same thing; with a hostname per service
-- it is a reason shown against names it was never about, which is worse
-- than no reason at all — the console would be confidently wrong.
--
-- It cannot live on `certificate` either. The failure that matters most
-- is "asked for a name, got nothing", and that is exactly the case
-- where no certificate row exists to carry it.
--
-- So it lands beside the policy, and `renew_with` becomes nullable to
-- keep the property that made that table honest: **absent is a value**.
-- A row that exists only to record a failure must not also assert a
-- choice nobody made — a stored default goes stale the day
-- `acme.disabled` changes, and would then say the opposite of what the
-- node does. NULL means "whatever the default resolves to now".
--
-- SQLite cannot drop a NOT NULL, so the table is rebuilt. It is one
-- migration old and holds a handful of rows.

ALTER TABLE certificate_policy RENAME TO certificate_policy_old;

CREATE TABLE certificate_policy (
    name       TEXT PRIMARY KEY,
    -- 'acme' | 'self_signed' | 'file', or NULL for the default.
    renew_with TEXT,
    cert_path  TEXT,
    key_path   TEXT,
    -- Why the last attempt for this name did not produce a
    -- certificate. NULL once one does.
    last_error TEXT,
    updated_at INTEGER NOT NULL
);

INSERT INTO certificate_policy
    ("name", "renew_with", "cert_path", "key_path", "last_error", "updated_at")
SELECT "name", "renew_with", "cert_path", "key_path", NULL, "updated_at"
FROM certificate_policy_old;

DROP TABLE certificate_policy_old;
