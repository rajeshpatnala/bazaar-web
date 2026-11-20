import { describe, expect, it } from "vitest";
import { formatIst, formatIstIso } from "./format-ist";

describe("formatIst", () => {
  it("renders IST regardless of browser zone", () => {
    // 2026-01-05T04:00:00Z is 09:30 IST
    expect(formatIst(Date.UTC(2026, 0, 5, 4, 0, 0) / 1000)).toBe("05 Jan, 09:30 IST");
  });
  it("shows midnight as 00:xx, never 24:xx", () => {
    // 2026-01-05T18:30:00Z is 00:00 IST on the 6th
    expect(formatIst(Date.UTC(2026, 0, 5, 18, 30, 0) / 1000)).toBe("06 Jan, 00:00 IST");
  });
  it("rolls the date over in IST", () => {
    // 2026-01-05T19:00:00Z is 00:30 IST on the 6th
    expect(formatIst(Date.UTC(2026, 0, 5, 19, 0, 0) / 1000)).toContain("06 Jan");
  });
});

describe("formatIstIso", () => {
  it("converts any offset to IST", () => {
    expect(formatIstIso("2026-11-19T10:00:00+05:30")).toBe("19 Nov, 10:00 IST");
    expect(formatIstIso("2026-11-19T04:30:00Z")).toBe("19 Nov, 10:00 IST");
  });
  it("rejects garbage", () => {
    expect(() => formatIstIso("nope")).toThrow();
  });
});
