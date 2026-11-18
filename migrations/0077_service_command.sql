-- What a service's container runs, when the operator says rather than the
-- image: Docker's two halves, each a JSON list of words, each NULL for the
-- image's own.
--
-- Two columns rather than one because an image is two: an `Entrypoint` that
-- prepares and a `Cmd` it is handed. Replacing only the `Cmd` keeps the
-- preparation — the `node` image's entrypoint is what makes `npm run worker`
-- work as typed — and an image whose entrypoint is the program itself needs
-- the other half replaced to run anything else.
ALTER TABLE service ADD COLUMN "entrypoint" TEXT;
ALTER TABLE service ADD COLUMN "command" TEXT;
