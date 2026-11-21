import { http, HttpResponse, type RequestHandler } from "msw";
import type { Overview } from "../features/overview/types";

/** Fake practice-account figures for the mocked UI. */
export const overviewFixture: Overview = {
  as_of: "2026-11-19T10:00:00+05:30",
  capital: "100000.00",
  day_pnl: "-350.00",
  open_positions: 1,
  ideas_today: 2,
  regime: "Sideways",
  data_delayed: false,
};

export const accountHandlers: RequestHandler[] = [
  http.get("/api/v1/overview", () => HttpResponse.json(overviewFixture)),
];
