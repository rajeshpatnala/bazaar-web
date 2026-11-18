import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { DISCLAIMER } from "../shared/disclaimer";
import { App } from "./App";

describe("App", () => {
  it("mounts the router with providers and shows the disclaimer", async () => {
    render(<App />);
    expect(await screen.findByText(DISCLAIMER)).toBeVisible();
  });
});
