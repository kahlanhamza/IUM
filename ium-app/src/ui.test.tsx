/**
 * Per-primitive a11y contracts (#66 Lane 1, #67): every ui.tsx variant is
 * audited by axe, so a screen assembled from these starts from an audited
 * baseline. A new primitive variant without an audit below fails nothing —
 * but the lane's acceptance bar (#66) is that new screens ship WITH their
 * audit; the table is deliberately explicit, not magic.
 */
// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import "axe-core";
import { Badge, Button, Chip, cx, Field, Select, TextInput } from "./components/ui";

declare global {
  interface Window {
    axe: {
      run: (
        el: HTMLElement,
        ctx?: Record<string, unknown>,
      ) => Promise<{ violations: { id: string; impact: string | null; nodes: unknown[] }[] }>;
    };
  }
}

async function expectAccessible(el: HTMLElement): Promise<void> {
  const { violations } = await window.axe.run(el, {
    rules: { "color-contrast": { enabled: false } },
  });
  const summary = violations
    .map((v) => `${v.id}[${v.impact}] x${v.nodes.length}`)
    .join(", ");
  expect(summary, `axe violations: ${summary}`).toBe("");
}

describe("cx joins truthy parts only", () => {
  it("drops false/null/undefined, keeps order", () => {
    expect(cx("btn", false && "x", null, undefined, "btn--sm", "")).toBe("btn btn--sm");
  });
});

describe("a11y: primitives", () => {
  it("Button — every variant × size", async () => {
    const { container } = render(
      <>
        <Button>Upload</Button>
        <Button variant="ghost" size="sm">
          Clear
        </Button>
        <Button variant="ghost" size="icon" aria-label="Pause job">
          ⏸
        </Button>
        <Button variant="danger-ghost" size="icon" aria-label="Cancel job">
          ✕
        </Button>
      </>,
    );
    await expectAccessible(container);
  });

  it("TextInput + Select + Field — labels are wired, not implied", async () => {
    const { container } = render(
      <>
        <Field label="Path" htmlFor="p" full>
          <TextInput id="p" mono />
        </Field>
        <Field label="Protocol" htmlFor="s">
          <Select id="s">
            <option value="tus">TUS 1.0.0</option>
          </Select>
        </Field>
        <TextInput aria-label="Standalone input" />
      </>,
    );
    await expectAccessible(container);
  });

  it("Chip + Badge — state travels in data-*, text stays human", async () => {
    const { container } = render(
      <div>
        <Chip state="running">running</Chip>
        <Chip>unknown</Chip>
        <Badge kind="resumable" title="Interruptions heal.">
          resumable
        </Badge>
        <Badge kind="plain">plain</Badge>
      </div>,
    );
    await expectAccessible(container);
  });
});
