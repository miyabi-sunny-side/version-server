import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import Icon from "./Icon.svelte";

function renderIcon(name: string): SVGSVGElement {
  const { container } = render(Icon, { props: { name } });
  const svg = container.querySelector("svg");
  if (!svg) {
    throw new Error(`icon "${name}" did not render an svg`);
  }
  return svg;
}

describe("Icon", () => {
  afterEach(cleanup);

  it("renders the screen icons as decorative SVGs with the shared grammar", () => {
    for (const name of ["menu", "x", "sun", "moon", "monitor"]) {
      const svg = renderIcon(name);
      expect(svg.getAttribute("viewBox"), name).toBe("0 0 24 24");
      expect(svg.getAttribute("fill"), name).toBe("none");
      expect(svg.getAttribute("stroke"), name).toBe("currentColor");
      expect(svg.getAttribute("stroke-width"), name).toBe("2");
      expect(svg.getAttribute("aria-hidden"), name).toBe("true");
      expect(svg.childElementCount, name).toBeGreaterThan(0);
      cleanup();
    }
  });
});
