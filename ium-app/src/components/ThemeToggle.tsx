/**
 * The Appearance control (#86): System / Light / Dark as one radiogroup.
 *
 * Real radio inputs — assistive tech speaks the group natively, arrow keys
 * move the selection, no ARIA is invented. Labels, not sentences (the
 * brand-sheet exempt class). This is the ONLY theme UI; the system
 * preference remains the default, so the control always shows three peers
 * with the current choice checked.
 */
import type { ThemePref } from "../theme";

const OPTIONS: Array<{ value: ThemePref; label: string }> = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

export function ThemeToggle({
  value,
  onChange,
}: {
  value: ThemePref;
  onChange: (pref: ThemePref) => void;
}) {
  return (
    <div className="theme-toggle" role="radiogroup" aria-label="Appearance">
      {OPTIONS.map((o) => (
        <label key={o.value} className="theme-option">
          <input
            type="radio"
            name="theme"
            value={o.value}
            checked={value === o.value}
            onChange={() => onChange(o.value)}
          />
          <span>{o.label}</span>
        </label>
      ))}
    </div>
  );
}
