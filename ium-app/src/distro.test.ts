/**
 * The distro gate (#101): the closed flip moved the distribution surface.
 * The source repo is PRIVATE; the updater feed + the downloads live on the
 * PUBLIC distro repo (kahlanhamza/IUM-dist). This gate pins the trust
 * surfaces: the baked updater endpoint must be the distro feed (the URL is
 * compiled into every shipped binary — tauri.conf.json at build time), and
 * the minisign pubkey MUST NOT change in a channel move (the updater's
 * trust root — a key rotation is a deliberate, reviewed, two-file act,
 * never an accident).
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const CONF_PATH = fileURLToPath(
  new URL("../src-tauri/tauri.conf.json", import.meta.url),
);
const conf = JSON.parse(readFileSync(CONF_PATH, "utf8"));

const DISTRO_FEED =
  "https://github.com/kahlanhamza/IUM-dist/releases/latest/download/latest.json";

// The v0.2.0/v0.2.1 trust root — pinned verbatim (#80 keypair). Rotating it
// must be a deliberate reviewed change to THIS constant too.
const UPDATER_PUBKEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDM2QjY1OUU5RUI2RjMxMkUKUldRdU1XL3I2Vm0yTmdtYUx3TlFQTUFzL1hRV3N5eWhacTBIZkRYb2dUSnNQTjB2UUZmelkySXEK";

describe("the distro channel (#101)", () => {
  it("the baked updater endpoint is the PUBLIC distro feed", () => {
    const endpoints = conf.plugins.updater.endpoints as string[];
    expect(endpoints).toContain(DISTRO_FEED);
  });

  it("no endpoint points at the private source repo", () => {
    const endpoints = conf.plugins.updater.endpoints as string[];
    const privateFeed = endpoints.filter((e) =>
      e.includes("github.com/kahlanhamza/IUM/"),
    );
    expect(
      privateFeed,
      "the source repo is private — its feed 404s for every user",
    ).toEqual([]);
  });

  it("the updater trust root (minisign pubkey) is unchanged by the channel move", () => {
    expect(conf.plugins.updater.pubkey).toBe(UPDATER_PUBKEY);
  });
});
