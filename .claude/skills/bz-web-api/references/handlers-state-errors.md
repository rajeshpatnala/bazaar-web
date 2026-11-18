# 5. Handlers, state, errors

(From `bz-web-api` SKILL.md; open when the task touches this area.)


```rust
#[utoipa::path(post, path = "/order-intents", tag = "orders",
    request_body = CreateOrderIntentRequest,
    responses((status = 201, body = OrderIntentDto), (status = 404, body = Problem, content_type = "application/problem+json"),
              (status = 409, body = Problem, content_type = "application/problem+json")))]
async fn create(State(s): State<AppState>, ApiJson(req): ApiJson<CreateOrderIntentRequest>)
    -> Result<(StatusCode, Json<OrderIntentDto>), ApiError>
{
    let created = s.orders.create_intent(req.recommendation_id).await?;   // port: intent -> gate -> approval queue
    Ok((StatusCode::CREATED, Json(OrderIntentDto::from(&created))))
}
pub fn router() -> OpenApiRouter<AppState> { OpenApiRouter::new().routes(routes!(create)) }
```

- `AppState` is `Clone` (Arcs): ports from `bazaar_core::ports` (recommendations, approvals/`ApprovalQueue`, kill switch, ledger, journal, candles, instruments, `MarginSource`, `Clock`, `SecretStore`), the `Hub`, `Jobs`, `security: Security` (guards take `State<Security>`, so pass `state.security.clone()` to `from_fn_with_state`), the server origin and `web_dist`. `GET /api/v1/auth/me` lives in `routes/auth.rs` with `#[utoipa::path]` and returns `AuthMeDto { csrf_token, mode, live_enabled }` (CONTRACTS section 4). It is built by `build_state(config, adapters)`; `main` passes real adapters, tests pass fakes. Never put a `sqlx` pool, `reqwest::Client` or `Executor` in a handler signature.
- `ApiJson`, `ApiQuery`, `ApiPath` wrap the axum extractors (`#[derive(FromRequest)] #[from_request(via(axum::Json), rejection(ApiError))]`) so malformed input becomes problem+json: syntax 400, schema 422, wrong content type 415, oversize 413.
- `ApiError` variants map one-to-one to problem `code`s (table in the reference). `From<ApprovalError>`, `From<GateError>` etc. are explicit matches, no catch-all; `Internal` logs once with `tracing::error!` and returns a generic detail. `?` on a domain error is the normal path.
- List routes cap `limit` server-side and use cursors (`{ items, next_cursor }`); instrument keys and enums are parsed into domain types before the port is called (unknown instrument is 404 `instrument_not_found`, bad enum 422).
