# Payments and commerce lens

Principle: the browser collects intent; the server decides money. The front end must be replaceable between Shopify, a headless stack, or a custom backend by swapping one adapter.

## 1. Trust boundaries (audit first)
- Client sends SKU/variant id + quantity only. Server prices the cart, applies discounts/tax/shipping, checks stock, then creates the order/payment intent. Flag any request body that carries price, total, discount, or "paid".
- "Order success" pages must be driven by server-verified payment (webhook / signature verification / retrieve intent), never by a `?success=1` URL or client callback alone.
- Idempotency keys on order/payment creation (double-click, retries, flaky mobile networks).
- Card data never touches your DOM or servers: hosted fields/iframe/redirect (Stripe Elements/Checkout, Razorpay Checkout, Shopify Checkout). That keeps PCI scope at SAQ A. No third-party scripts on payment pages beyond the gateway.
- Webhooks: verify signature, idempotent handlers, replay-safe. (Server concern; flag when the repo shows none.)

## 2. Option guide (give a recommendation, not a survey)
| Need | Recommend |
|---|---|
| Few products, fastest, safest | Shopify (Buy Button / Storefront API cart + Shopify-hosted checkout) |
| Custom storefront UI, Shopify catalog | Storefront API (GraphQL) + Shopify checkout redirect; Hydrogen if React/Remix is welcome |
| Full control, India payments (UPI/cards/netbanking) | Razorpay or Cashfree Orders API + server-created order + signature verify |
| Global cards/wallets, subscriptions | Stripe Checkout (hosted) first; Elements only if custom UI is required |
| Static site, no backend | Hosted payment links/Checkout + serverless functions for order creation and webhooks |
Prefer hosted/redirect checkout unless brand needs justify the extra PCI and security burden.

## 3. Seam design (demo → real, one-place swap)
```js
// contract the UI depends on; implement per backend
CatalogService: list(), get(handle)
CartService:    get(), add(variantId, qty), update(lineId, qty), remove(lineId), subscribe(fn)
CheckoutService: start(cart, customer) → { redirectUrl | clientSecret }, confirm(token)
```
Adapters: `mock` (current demo), `shopify` (Storefront API cart: `cartCreate`, `cartLinesAdd`, `checkoutUrl`), `custom` (your REST/GraphQL). Selected by config, never by editing UI files. Keep `DUMMY` data behind the mock adapter only and make production builds fail if the mock is selected.

## 4. Cart and checkout UX/robustness checklist
- Cart persisted (versioned), reconciled with server on load (price/stock changes shown, not silently applied).
- Quantity limits, out-of-stock and variant availability handled; optimistic UI with rollback on error.
- Currency/tax/shipping display from server data; `Intl.NumberFormat`; integer minor units.
- Address/phone/pincode validation per locale, accessible inline errors, `autocomplete` tokens, `inputmode`.
- Loading, empty, error, offline and retry states for every network call; timeouts; no double submit (disable + idempotency).
- Guest checkout path; order confirmation + email handled server-side; order lookup uses unguessable ids, not sequential numbers.
- Analytics events (view_item, add_to_cart, begin_checkout, purchase) via one adapter, consent-gated, no PII.
- Policy pages (refund, shipping, privacy, terms) linked in footer and checkout; required by gateways and consumer law.

## 5. Shopify specifics
- Storefront API token is public-by-design but scope it minimal; Admin API token must never be in the front end.
- Use `checkoutUrl` from the cart for payment; do not rebuild checkout. Pass discount codes via cart API.
- Theme/Liquid: escape with `| escape`, avoid `| raw`-style unsafe output, never inline unreviewed app scripts; audit installed apps' injected scripts.
- Webhooks and metafields for content; avoid hardcoding product handles/ids in JS (use metafields/collections).

## 6. Red flags to always report
Prices in JS/DOM used for charging; success redirect without verification; secret keys or admin tokens in repo; sequential order ids exposed; payment page loading tag managers/chat widgets; `localStorage` holding PII/tokens; demo checkout reachable in production.
