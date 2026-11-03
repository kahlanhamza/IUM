// server.mjs — the second TUS lane's server (issue #33).
//
// @tus/server (tus-node-server) + @tus/file-store, PINNED versions in
// package.json (+ package-lock.json = the supply-chain guard). Boots a TUS
// 1.0.0 server on 127.0.0.1:<port> serving <data-dir> — core + creation +
// termination/expiration (whatever the store declares), NO tusd GET-download
// extension (tus 1.0.0 core has no download; that extension is tusd-only and
// the tests that need it honestly skip on this lane).
//
// Usage: node server.mjs <data-dir> <port>
import { Server } from "@tus/server";
import { FileStore } from "@tus/file-store";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const [dataDir, portArg] = process.argv.slice(2);
if (!dataDir || !portArg) {
  console.error("usage: node server.mjs <data-dir> <port>");
  process.exit(2);
}
const directory = resolve(dataDir);
mkdirSync(directory, { recursive: true });
const port = Number(portArg);

const server = new Server({
  path: "/files/",
  datastore: new FileStore({ directory }),
});

server.listen({ host: "127.0.0.1", port }, () => {
  console.log(`[tus-node-lane] listening on 127.0.0.1:${port}/files/`);
});
