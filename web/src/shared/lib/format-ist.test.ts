import { describe, expect, it } from "vitest";
import { formatIst } from "./format-ist";

describe("formatIst", () => {
  it("renders IST regardless of browser zone", () => {
    // 2026-01-05T04:00:00Z is 09:30 IST
    expect(formatIst(Date.UTC(2026, 0, 5, 4, 0, 0) / 1000)).toBe("05 Jan, 09:30 IST");
  });
  it("rolls the date over in IST", () => {
    // 2026-01-05T19:00:00Z is 00:30 IST on the 6th
    expect(formatIst(Date.UTC(2026, 0, 5, 19, 0, 0) / 1000)).toContain("06 Jan");
  });
});
