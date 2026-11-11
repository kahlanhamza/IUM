/**
 * The provider registry loader (the destination lane, owner-ratified
 * 2026-11-11: regular users think in websites, not path-style object
 * URLs). The JSON in `data/providers.json` is the SINGLE source of truth —
 * `ium-core/tests/providers_registry.rs` feeds every preset's sample URL
 * through the ENGINE's own `parse_target`, so anything this module
 * derives is a destination the engine accepts by construction. This file
 * adds the derivation math and the custom-mode escape hatch: power users
 * keep the raw field (the default), presets are additive.
 *
 * No engine changes ride this lane (ADR-012): provider knowledge is DATA
 * in the app; adapters declare, the planner obeys.
 */
import raw from "./data/providers.json";

export interface ProviderSlot {
  name: string;
  label: string;
  kind: "text" | "enum";
  /** enum only */
  options?: string[];
  /** enum only */
  default?: string;
  /** text only */
  placeholder?: string;
}

export interface Provider {
  id: string;
  label: string;
  adapter: string;
  endpoint_template: string;
  regions?: string[];
  region_default?: string;
  slots?: ProviderSlot[];
  key_page: string | null;
  key_steps: string;
  note?: string;
  self_hosted?: boolean;
}

interface Registry {
  version: number;
  providers: Provider[];
}

export const PROVIDER_CUSTOM = "custom";

const registry = raw as unknown as Registry;

/** The real presets (no pseudo-entries). */
export function loadProviders(): Provider[] {
  return registry.providers;
}

export function providerById(id: string): Provider | undefined {
  return registry.providers.find((p) => p.id === id);
}

/**
 * The picker's option list for a lane: "Custom URL" first (the default,
 * byte-compatible with today's raw field), then the presets. Lanes with
 * no presets get an empty list — the form then renders custom-only and
 * behaves exactly as before this lane existed.
 */
export function providersForAdapter(adapter: string): Provider[] {
  const presets = registry.providers.filter((p) => p.adapter === adapter);
  const custom: Provider = {
    id: PROVIDER_CUSTOM,
    label: "Custom URL (paste any destination)",
    adapter,
    endpoint_template: "",
    key_page: null,
    key_steps: "",
  };
  return [custom, ...presets];
}

/**
 * The guided inputs a preset asks for: the {region} slot (enum, from
 * `regions`/`region_default` when the template needs one) plus any
 * explicit text slots (the R2 account id, a MinIO base URL). The bucket
 * is NOT a slot — every preset asks for it separately.
 */
export function guidedSlots(p: Provider): ProviderSlot[] {
  const out: ProviderSlot[] = [];
  if (p.endpoint_template.includes("{region}")) {
    out.push({
      name: "region",
      label: "Region",
      kind: "enum",
      options: p.regions ?? [],
      default: p.region_default,
    });
  }
  for (const s of p.slots ?? []) out.push(s);
  return out;
}

/**
 * The derivation math: fill every template slot from `values`, append
 * `/bucket`. Incomplete input derives to "" (the form simply isn't ready)
 * — never a half-URL the user would have to hand-fix. Values not named by
 * a slot are ignored, never smuggled into the URL.
 */
export function buildEndpoint(p: Provider, values: Record<string, string>): string {
  if (p.id === PROVIDER_CUSTOM) return "";
  let out = p.endpoint_template;
  for (const s of guidedSlots(p)) {
    const v = (values[s.name] ?? "").trim();
    if (!v) return "";
    out = out.split(`{${s.name}}`).join(v);
  }
  const bucket = (values.bucket ?? "").trim();
  if (!bucket) return "";
  return `${out}/${bucket}`;
}

export function canDerive(p: Provider, values: Record<string, string>): boolean {
  return buildEndpoint(p, values) !== "";
}
