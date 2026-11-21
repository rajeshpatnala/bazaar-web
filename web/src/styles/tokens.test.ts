import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const css = readFileSync("src/styles/tokens.css", "utf8");
const token = (name: string): string => {
  const m = new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`).exec(css);
  if (!m?.[1]) throw new Error(`token ${name} missing`);
  return m[1];
};

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * (r ?? 0) + 0.7152 * (g ?? 0) + 0.0722 * (b ?? 0);
}
function ratio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return ((hi ?? 0) + 0.05) / ((lo ?? 0) + 0.05);
}

describe("token contrast (WCAG AA, 4.5:1)", () => {
  it.each([
    ["ink", "bg"],
    ["ink", "card"],
    ["muted", "bg"],
    ["muted", "card"],
    ["green", "green-bg"],
    ["red", "red-bg"],
    ["amber", "card"],
    ["amber", "amber-bg"],
    ["on-accent", "green"],
  ])("%s on %s", (fg, bg) => {
    expect(ratio(token(fg), token(bg))).toBeGreaterThanOrEqual(4.5);
  });
});
