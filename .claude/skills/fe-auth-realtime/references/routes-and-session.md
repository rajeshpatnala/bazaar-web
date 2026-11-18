# Protected routes and session sync

Route guards, login page, safe redirects, session expiry and multi-tab sync.

## Contents

- Protected routes, login page and safe redirects
- Session expiry and multi-tab sync

## Protected routes, login page and safe redirects

One pathless layout route guards every signed-in page. Its `beforeLoad` runs before any child loader, so a signed-out user never triggers protected API calls or sees a flash of protected UI. They are redirected to `/login` with the page they wanted saved in `?redirect=`.

**Changes to `src/app/router.tsx`** (`fe-app-architecture`'s routes move under the guard):

```tsx
import { notFound, redirect } from '@tanstack/react-router';
import { z } from 'zod';
import { can, safeRedirect, sessionQuery } from '@/features/auth';
import { AuthenticatedLayout } from './authenticated-layout';

const authenticatedRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: 'authenticated', // pathless: adds a guard, not a URL segment
  beforeLoad: async ({ context, location }) => {
    const session = await context.queryClient.ensureQueryData(sessionQuery);
    if (!session) throw redirect({ to: '/login', search: { redirect: location.href } });
    return { session }; // typed `context.session` in every child route
  },
  component: AuthenticatedLayout,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/login',
  validateSearch: z.object({ redirect: z.string().optional().catch(undefined) }),
  beforeLoad: async ({ context, search }) => {
    const session = await context.queryClient.ensureQueryData(sessionQuery);
    if (session) throw redirect({ href: safeRedirect(search.redirect) }); // already signed in
  },
  component: lazyRouteComponent(() => import('@/pages/login-page'), 'LoginPage'),
});

const adminRoute = createRoute({
  getParentRoute: () => authenticatedRoute,
  path: '/admin',
  // 404, not 403: don't reveal that an admin area exists
  beforeLoad: ({ context }) => {
    if (!can(context.session, 'admin:access')) throw notFound();
  },
  component: lazyRouteComponent(() => import('@/pages/admin-page'), 'AdminPage'),
});

// ordersRoute and orderDetailRoute now use: getParentRoute: () => authenticatedRoute

const routeTree = rootRoute.addChildren([
  homeRoute,
  loginRoute,
  authenticatedRoute.addChildren([ordersRoute, orderDetailRoute, adminRoute]),
]);
```

`ensureQueryData` reuses the cached session (fresh for 5 minutes), so the guard costs no request on most navigations.

**`src/app/authenticated-layout.tsx`** — starts real-time updates for signed-in users (completed in the real-time section):

```tsx
import { Outlet } from '@tanstack/react-router';

export function AuthenticatedLayout() {
  return <Outlet />;
}
```

**`features/auth/components/login-form.tsx`**

```tsx
import { useForm } from 'react-hook-form';
import { zodResolver } from '@hookform/resolvers/zod';
import { HttpError } from '@/shared/lib/http';
import { toUserMessage } from '@/shared/lib/errors';
import { useLogin } from '../api/auth-api';
import { LoginInput } from '../model/session';

function loginErrorMessage(error: unknown): string {
  // One message for unknown email and wrong password: never reveal which accounts exist.
  if (error instanceof HttpError && error.status === 401) return 'Email or password is incorrect.';
  return toUserMessage(error);
}

export function LoginForm({ onSuccess }: { onSuccess: () => void }) {
  const login = useLogin();
  const { register, handleSubmit, formState: { errors } } = useForm<LoginInput>({ resolver: zodResolver(LoginInput) });

  return (
    <form onSubmit={handleSubmit((values) => login.mutate(values, { onSuccess }))} noValidate>
      <label htmlFor="email">Email</label>
      <input
        id="email"
        type="email"
        autoComplete="username"
        aria-invalid={Boolean(errors.email)}
        aria-describedby={errors.email ? 'email-error' : undefined}
        {...register('email')}
      />
      {errors.email && <p id="email-error">{errors.email.message}</p>}

      <label htmlFor="password">Password</label>
      <input
        id="password"
        type="password"
        autoComplete="current-password"
        aria-invalid={Boolean(errors.password)}
        aria-describedby={errors.password ? 'password-error' : undefined}
        {...register('password')}
      />
      {errors.password && <p id="password-error">{errors.password.message}</p>}

      {login.isError && <p role="alert">{loginErrorMessage(login.error)}</p>}
      <button type="submit" disabled={login.isPending}>{login.isPending ? 'Signing in…' : 'Sign in'}</button>
    </form>
  );
}
```

**`src/pages/login-page.tsx`**

```tsx
import { useRouter, useSearch } from '@tanstack/react-router';
import { LoginForm, safeRedirect } from '@/features/auth';

export function LoginPage() {
  const { redirect } = useSearch({ from: '/login' });
  const router = useRouter();

  return (
    <section aria-labelledby="login-heading">
      <h1 id="login-heading">Sign in</h1>
      <LoginForm onSuccess={() => router.history.push(safeRedirect(redirect))} />
    </section>
  );
}
```

The correct `autoComplete` values let password managers fill and save credentials. When you add passkeys later, use `autoComplete="username webauthn"` on the email field.

## Session expiry and multi-tab sync

One module reacts to auth changes from anywhere, whether a failed refresh in this tab or a sign-in or sign-out in another tab. When a session ends, the app clears every cached response before showing the login page, so the next person to sign in on that device never sees the previous user's data.

**`src/app/auth-sync.ts`**

```ts
import type { QueryClient } from '@tanstack/react-query';
import { sessionQuery } from '@/features/auth';
import { authEvents } from '@/shared/lib/http';
import { onAuthBroadcast } from '@/shared/lib/auth-channel';
import type { AppRouter } from './router';

export function startAuthSync(queryClient: QueryClient, router: AppRouter): () => void {
  const onExpired = () => {
    // Ignore the anonymous first load: there was no session to expire.
    if (!queryClient.getQueryData(sessionQuery.queryKey)) return;
    queryClient.clear(); // drop all of the previous user's data
    queryClient.setQueryData(sessionQuery.queryKey, null);
    void router.invalidate(); // re-runs beforeLoad → /login?redirect=<current page>
  };
  authEvents.addEventListener('session-expired', onExpired);

  const stopBroadcast = onAuthBroadcast((event) => {
    if (event === 'logout') {
      window.location.assign('/login'); // signed out elsewhere: reload clears everything
    } else {
      void queryClient.invalidateQueries({ queryKey: sessionQuery.queryKey }).then(() => router.invalidate());
    }
  });

  return () => {
    authEvents.removeEventListener('session-expired', onExpired);
    stopBroadcast();
  };
}
```

**Wire it in `src/main.tsx`**, after creating the router:

```ts
import { startAuthSync } from '@/app/auth-sync';

startAuthSync(queryClient, router);
```

**Behaviour summary**

| Event | Result |
| --- | --- |
| Access cookie expires during use | Silent refresh; the user notices nothing |
| Refresh token expired or revoked | Cache cleared → login page → back to the same page after sign-in |
| Sign-out in another tab | This tab reloads to `/login` |
| Sign-in in another tab | This tab picks up the new session without a reload |
| Tab regains focus after a while | TanStack Query refetches the stale session (`refetchOnWindowFocus`), so a revoked session is noticed |

For long forms, persist the draft (a Zustand store with `persist`, `fe-app-architecture`) so an expired session never costs the user their work.
