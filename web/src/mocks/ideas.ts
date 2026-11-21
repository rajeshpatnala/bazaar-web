import { http, HttpResponse, type RequestHandler } from "msw";
import type { Idea, IdeasResponse } from "../features/ideas/types";

/** Fake data for the mocked UI; not a real recommendation. */
export const ideasFixture: Idea[] = [
  {
    id: "idea-1",
    symbol: "RELIANCE",
    side: "buy",
    entry: "2850.00",
    stop: "2800.00",
    target: "2950.00",
    quantity: 20,
    max_loss: "1000.00",
    low_evidence: false,
    regime: "Trending up",
    data_delayed: false,
    why: [
      "The price has been moving up for several days.",
      "If it falls to the stop price, the idea is wrong and you would exit with a small loss.",
    ],
  },
  {
    id: "idea-2",
    symbol: "TCS",
    side: "sell",
    entry: "4100.00",
    stop: "4160.00",
    target: "3980.00",
    quantity: 10,
    max_loss: "600.00",
    low_evidence: true,
    regime: "Sideways",
    data_delayed: true,
    why: ["The price is near the top of its recent range.", "Few past examples match, so treat this with extra care."],
  },
];

export const ideasResponseFixture: IdeasResponse = { ideas: ideasFixture, as_of: "2026-11-19T10:00:00+05:30" };

export const ideasHandlers: RequestHandler[] = [
  http.get("/api/v1/ideas", () => HttpResponse.json(ideasResponseFixture)),
];
