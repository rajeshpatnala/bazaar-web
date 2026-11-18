import { useState } from "react";
import { createBrowserRouter } from "react-router";
import { RouterProvider } from "react-router/dom";
import { Providers, makeQueryClient } from "./providers";
import { routes } from "./routes";

export function App() {
  const [client] = useState(makeQueryClient);
  const [router] = useState(() => createBrowserRouter(routes));
  return (
    <Providers client={client}>
      <RouterProvider router={router} />
    </Providers>
  );
}
