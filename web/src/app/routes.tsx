import type { RouteObject } from "react-router";
import { IdeasPage } from "../features/ideas/IdeasPage";
import { AppShell } from "./AppShell";
import { Home } from "./Home";

function RouteError() {
  return (
    <p role="alert">
      Something went wrong showing this page. Nothing has been ordered. Please reload the page or
      go back to Home.
    </p>
  );
}

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    children: [
      {
        // Pathless layout route: a render-time throw is caught here, inside AppShell,
        // so the header and the permanent disclaimer stay on screen.
        errorElement: <RouteError />,
        children: [
          { index: true, element: <Home /> },
          { path: "ideas", element: <IdeasPage /> },
        ],
      },
    ],
  },
  { path: "*", element: <AppShell />, children: [{ path: "*", element: <p>Page not found.</p> }] },
];
