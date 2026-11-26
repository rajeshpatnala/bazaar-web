import { render } from "@testing-library/react";
import { createMemoryRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Providers, makeQueryClient } from "../app/providers";
import { routes } from "../app/routes";

export function renderApp(opts: { route?: string } = {}) {
  const router = createMemoryRouter(routes, { initialEntries: [opts.route ?? "/"] });
  const client = makeQueryClient();
  const view = render(
    <Providers client={client}>
      <RouterProvider router={router} />
    </Providers>,
  );
  return { ...view, client };
}
