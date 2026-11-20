import type { RouteObject } from "react-router";
import { IdeasPage } from "../features/ideas/IdeasPage";
import { AppShell } from "./AppShell";
import { Home } from "./Home";

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <Home /> },
      { path: "ideas", element: <IdeasPage /> },
    ],
  },
  { path: "*", element: <AppShell />, children: [{ path: "*", element: <p>Page not found.</p> }] },
];
