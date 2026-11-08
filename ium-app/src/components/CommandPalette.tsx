/**
 * The ⌘K command palette (#91) — the keyboard-first lane.
 *
 * The approved mockup (design/mockup/palette-*.html) becomes behavior: a
 * 640 px surface 96 px from the window top over the smoke scrim, 36 px rows
 * in the content-row rhythm, the ink active row, JetBrains Mono keycaps, the
 * hairline footer. Same surface family as the Settings modal — the shadow
 * law's floating-surface exception, the 4–10 px radii, zero new deps.
 *
 * Honesty rule (#91): the palette renders only commands the caller wired to
 * real APIs. The two mockup rows with no engine backing never shipped (see
 * the issue's command table) — a command that cannot run must not render.
 *
 * Keyboard contract: ⌘K/Ctrl+K toggles (owned by App), esc closes, ↑/↓ move
 * with wrap, ↵ runs, typing filters case-insensitively, and the row keycaps
 * fire while the palette is open. Focus lands in the filter input on open
 * and returns to the summoner on close.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { IconSearch } from "./icons";

export interface Command {
  id: string;
  label: string;
  group: "Actions" | "Go to";
  /** Keycap pair rendered on the row AND honored while the palette is open. */
  keys?: [string, string];
  run: () => void;
}

/** The platform-true modifier glyph: ⌘ on macOS, Ctrl elsewhere. */
export function modGlyph(): string {
  return /Mac/i.test(navigator.platform) ? "⌘" : "Ctrl";
}

const GROUPS: Command["group"][] = ["Actions", "Go to"];

export function CommandPalette(props: {
  open: boolean;
  onClose: () => void;
  commands: Command[];
}) {
  const { open, onClose, commands } = props;
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const summoner = useRef<HTMLElement | null>(null);

  const filtered = useMemo(
    () =>
      commands.filter((c) => c.label.toLowerCase().includes(query.toLowerCase())),
    [commands, query],
  );

  // Focus lands in the input on open; the summoner gets focus back on close.
  useEffect(() => {
    if (open) {
      summoner.current = document.activeElement as HTMLElement | null;
      setQuery("");
      setActive(0);
      inputRef.current?.focus();
    } else {
      summoner.current?.focus();
    }
  }, [open]);

  if (!open) return null;

  const runAt = (i: number) => {
    const cmd = filtered[i];
    if (!cmd) return;
    onClose();
    cmd.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (filtered.length === 0) return;
      const delta = e.key === "ArrowDown" ? 1 : -1;
      setActive((cur) => (cur + delta + filtered.length) % filtered.length);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      runAt(active);
      return;
    }
    // The row keycaps fire while the palette is open (⌘1 / ⌘3 / ⌘, / ⌘N).
    if (e.metaKey || e.ctrlKey) {
      const hit = commands.find(
        (c) => c.keys && c.keys[1].toLowerCase() === e.key.toLowerCase(),
      );
      if (hit) {
        e.preventDefault();
        onClose();
        hit.run();
      }
    }
  };

  return (
    <>
      <div className="cmdk-scrim" onClick={onClose} aria-hidden="true" />
      <div
        className="cmdk"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
      >
        <div className="cmdk-input">
          <IconSearch size={16} />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
            role="textbox"
            aria-label="Filter commands"
            aria-controls="cmdk-list"
            aria-activedescendant={
              filtered.length > 0 ? `cmdk-opt-${active}` : undefined
            }
            autoComplete="off"
            spellCheck={false}
          />
          <span className="keycap">esc</span>
        </div>

        <div className="cmdk-list" id="cmdk-list" role="listbox" aria-label="Commands">
          {GROUPS.map((group) => {
            const rows = filtered
              .map((c, i) => ({ c, i }))
              .filter(({ c }) => c.group === group);
            if (rows.length === 0) return null;
            return (
              <div key={group} className="cmdk-group">
                <div className="cmdk-label">{group}</div>
                {rows.map(({ c, i }) => {
                  return (
                    <div
                      key={c.id}
                      id={`cmdk-opt-${i}`}
                      role="option"
                      aria-selected={i === active}
                      className={`cmdk-row${i === active ? " active" : ""}`}
                      onMouseEnter={() => setActive(i)}
                      onClick={() => runAt(i)}
                    >
                      <span className="cmdk-row-label">{c.label}</span>
                      {c.keys && (
                        <span className="cmdk-keys">
                          <span className="keycap">{c.keys[0] + c.keys[1]}</span>
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            );
          })}
          {filtered.length === 0 && (
            <div className="cmdk-empty">No matching commands</div>
          )}
        </div>

        <div className="cmdk-foot">
          <span>↑↓ navigate</span>
          <span>↵ run</span>
          <span>esc close</span>
        </div>
      </div>
    </>
  );
}
