/**
 * The settings logic (#69): pure presentation math between the masked IPC
 * view and the form state. No Tauri imports — testable in plain vitest.
 *
 * The secret rule the whole module enforces: credential VALUES never reach
 * the UI (the command returns `value: null, present: true` for secrets),
 * and only EXPLICIT edits travel back. A draft is `null` (untouched — not
 * sent), a string (new value — sent), or `""` (explicit clear — sent, and
 * the vault deletes the field).
 */

export interface FieldView {
  label: string;
  secret: boolean;
  required: boolean;
  present: boolean;
  value: string | null;
}

export type AdapterViews = Record<string, Record<string, FieldView>>;

export interface FieldRow {
  name: string;
  label: string;
  secret: boolean;
  required: boolean;
  /** A value is stored in the vault (secrets: the value itself is unknown here). */
  present: boolean;
  /** Convenience mirror of `present` — the chip the UI renders. */
  saved: boolean;
  /** The stored value — secrets carry null by contract. */
  value: string | null;
  /** null = untouched; "" = explicit clear; otherwise a new value. */
  draft: string | null;
}

export function toRows(views: AdapterViews): Record<string, FieldRow[]> {
  const out: Record<string, FieldRow[]> = {};
  for (const [adapter, fields] of Object.entries(views)) {
    out[adapter] = Object.entries(fields).map(([name, f]) => ({
      name,
      label: f.label,
      secret: f.secret,
      required: f.required,
      present: f.present,
      saved: f.present,
      value: f.value,
      draft: null,
    }));
  }
  return out;
}

/** Only explicit edits travel: untouched rows are omitted entirely. */
export function changedFields(rows: FieldRow[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const r of rows) {
    if (r.draft !== null) out[r.name] = r.draft;
  }
  return out;
}

/** A required field must end up present-or-drafted; clearing one blocks save. */
export function canSave(rows: FieldRow[]): boolean {
  return rows.every((r) => {
    if (!r.required) return true;
    if (r.draft === "") return false; // explicit clear of a required field
    if (r.draft !== null) return true; // a value is being provided
    return r.present; // untouched: fine iff already saved
  });
}
