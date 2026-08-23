-- Who else answers for a name, and the secret they use to reach each
-- other.
--
-- **For the HTTP-01 challenge, and nothing else.** A public authority
-- fetches the token from whichever address the name resolves to, and the
-- token lives in the ordering node's own `acme_challenge` table — so a
-- name several nodes answer for is a name the authority may ask the wrong
-- one for. Each miss is one of five failed validations an hour and the
-- lock is per account, which is why this exists rather than "try and see".
--
-- Both tables are written from an **edge errand**: the node that owns the
-- service is the only one that knows the whole set, and two nodes
-- enrolled by the same authority have never heard of each other — see
-- `docs/network.md`. The owner writes its own rows directly, from
-- `edges::set`.
CREATE TABLE IF NOT EXISTS edge_peer (
  "hostname"   TEXT    NOT NULL,
  "node_id"    TEXT    NOT NULL,
  "updated_at" INTEGER NOT NULL,
  PRIMARY KEY ("hostname", "node_id")
);

-- One secret per name, minted by the owner and handed to every edge of
-- it.
--
-- It authorizes exactly one thing: writing a challenge for that one name.
-- Held in clear because every holder has to *present* it as well as check
-- it — a hash would let a node verify and never speak — and what it can
-- do is bounded by the name it belongs to.
--
-- **Why it is needed at all**: two edges of one name may have no
-- relationship. The overlay authenticates the channel (a WireGuard peer
-- cannot forge another's address) but nothing above it says who called,
-- and an endpoint that writes challenge answers must not be open to
-- whoever reaches it: an attacker who could plant a response would get a
-- certificate for somebody else's name.
CREATE TABLE IF NOT EXISTS edge_secret (
  "hostname"   TEXT PRIMARY KEY,
  "secret"     TEXT    NOT NULL,
  "updated_at" INTEGER NOT NULL
);
