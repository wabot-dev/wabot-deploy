-- What a passkey is called on Your account › Passkeys.
--
-- `aaguid` is the authenticator's model, which names the password manager
-- or key that holds it. A passkey made before this has none, and nothing
-- can recover it: only the ceremony that makes one carries it.
--
-- `label` is the name its owner gave it, and NULL until they do.
--
-- `transports` is how the browser reached it — `usb`, `nfc`, `internal`,
-- `hybrid` — comma-separated, which is what tells a security key from a
-- passkey that never left the device.
ALTER TABLE passkey ADD COLUMN aaguid TEXT;
ALTER TABLE passkey ADD COLUMN label TEXT;
ALTER TABLE passkey ADD COLUMN transports TEXT NOT NULL DEFAULT '';
