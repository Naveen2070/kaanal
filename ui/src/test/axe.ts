// axe-core (dev only, never shipped) over a rendered component. jsdom has no layout, so colour
// contrast is off here: the tokens are checked by theme/contrast.test.ts. `region` (all content in a
// landmark) is only meaningful for a whole page, so a fragment turns it off.

import axe from "axe-core";
import { expect } from "vite-plus/test";

export async function expectNoViolations(
  container: Element,
  { page = false }: { page?: boolean } = {},
): Promise<void> {
  const result = await axe.run(container, {
    rules: { "color-contrast": { enabled: false }, region: { enabled: page } },
  });
  expect(
    result.violations.map(
      (v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).join(", ")})`,
    ),
  ).toEqual([]);
}
