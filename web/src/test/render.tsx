import { render } from "@testing-library/react";
import { createMemoryRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Providers, makeQueryClient } from "../app/providers";
import { routes } from "../app/routes";

export function renderApp(opts: { route?: string } = {}) {
  const router = createMemoryRouter(routes, { initialEntries: [opts.route ?? "/"] });
  return render(
    <Providers client={makeQueryClient()}>
      <RouterProvider router={router} />
    </Providers>,
  );
}
