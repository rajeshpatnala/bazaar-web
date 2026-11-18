import { describe, expect, it } from "vitest";
import { formatInr } from "./format-inr";

describe("formatInr", () => {
  it.each([
    ["0", "₹0.00"],
    ["999", "₹999.00"],
    ["1000", "₹1,000.00"],
    ["99999", "₹99,999.00"],
    ["100000", "₹1,00,000.00"],
    ["12345678", "₹1,23,45,678.00"],
    ["-1234.5", "-₹1,234.50"],
    ["1234.567", "₹1,234.57"],
    ["1234.564", "₹1,234.56"],
    ["1234.565", "₹1,234.56"],
    ["1234.575", "₹1,234.58"],
    ["1234.5650001", "₹1,234.57"],
    ["0.995", "₹1.00"],
    ["999999.999", "₹10,00,000.00"],
    ["-0.001", "₹0.00"],
  ])("formats %s", (input, expected) => expect(formatInr(input)).toBe(expected));

  it.each([
    ["1500.75", "₹1,501"],
    ["1500.5", "₹1,500"],
    ["1501.5", "₹1,502"],
  ])("rounds %s to zero decimals", (input, expected) =>
    expect(formatInr(input, { decimals: 0 })).toBe(expected),
  );
  it("rejects non-numeric input", () => expect(() => formatInr("abc")).toThrow());
});
