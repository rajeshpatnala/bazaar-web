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
    ["1234.567", "₹1,234.56"],
  ])("formats %s", (input, expected) => expect(formatInr(input)).toBe(expected));

  it("supports zero decimals", () => expect(formatInr("1500.75", { decimals: 0 })).toBe("₹1,500"));
  it("rejects non-numeric input", () => expect(() => formatInr("abc")).toThrow());
});
