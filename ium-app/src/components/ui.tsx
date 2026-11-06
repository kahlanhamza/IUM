/**
 * UI primitives (#66 Lane 1, #67): the design system as typed components.
 *
 * Each primitive composes the EXISTING theme.css token classes — visual
 * output is byte-identical to the #65 Lane 1 pass; no CSS is rewritten.
 * The a11y gate (src/a11y.test.tsx) audits every variant, so a new screen
 * built from these inherits an audited baseline instead of re-rolling
 * accessibility by hand.
 *
 * The pinned app-e2e seams are passthrough props: ids (`#upload-*`),
 * adapter option VALUES (registry names), `data-state` chips, and the
 * semantic table structure all arrive at the real DOM untouched.
 */
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  SelectHTMLAttributes,
} from "react";

/** Join truthy class parts — the only className logic this kit allows. */
export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ");
}

export type ButtonVariant = "primary" | "ghost" | "danger-ghost";
export type ButtonSize = "sm" | "icon";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export function Button({ variant = "primary", size, className, type, ...rest }: ButtonProps) {
  return (
    <button
      type={type ?? "button"}
      className={cx("btn", `btn--${variant}`, size && `btn--${size}`, className)}
      {...rest}
    />
  );
}

export interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  /** Monospace input (paths, endpoints) — a content decision, not a style whim. */
  mono?: boolean;
}

export function TextInput({ mono, className, ...rest }: TextInputProps) {
  return <input className={cx("input", mono && "mono", className)} spellCheck={false} {...rest} />;
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={cx("select", className)} {...rest}>
      {children}
    </select>
  );
}

export interface FieldProps {
  label: string;
  htmlFor: string;
  /** Full-width field (spans the form grid) — the path field is the one today. */
  full?: boolean;
  children: ReactNode;
}

export function Field({ label, htmlFor, full, children }: FieldProps) {
  return (
    <div className={cx("field", full && "field--full")}>
      <label className="field-label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
    </div>
  );
}

export interface ChipProps {
  /** Journal state as a machine-readable attribute — the #35/#65 e2e seam. */
  state?: string;
  children: ReactNode;
}

export function Chip({ state, children }: ChipProps) {
  return (
    <span className="chip" data-state={state}>
      {children}
    </span>
  );
}

export interface BadgeProps {
  kind: string;
  /** Hover tooltip explaining the capability in a sentence. */
  title?: string;
  children: ReactNode;
}

export function Badge({ kind, title, children }: BadgeProps) {
  return (
    <span className={cx("badge", kind === "resumable" && "badge--resumable")} title={title}>
      {children}
    </span>
  );
}
