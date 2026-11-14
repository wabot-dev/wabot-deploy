-- SSH, opened to everyone, which is what it was. Its own migration rather
-- than a line in 0068: SSH is guarded in the kernel, which came after the
-- edge, and without this row the first pass that writes the rules would
-- close the operator's own shell.
INSERT OR IGNORE INTO allowlist (door, subject, network, created_at)
VALUES ('ssh', '', '0.0.0.0/0', 0), ('ssh', '', '::/0', 0);
