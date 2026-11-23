# Top 10: dev-api (from 38 findings, 2026-11-23)
1. Every spawned task has its handle held and supervised: a panic or early return stops the server or raises an alarm [RISK-142-1, SEC-142-1].
2. Every `pub` runtime method has a production caller (grep it); a method only tests call is not wiring [RISK-142-2].
3. Wiring tests use the production base inputs and fail when the guard under test is released (flip it and watch the test fail) [RISK-138-1, CR-138-1].
4. A config/constant test never compares a value with the constant it was written from [RISK-138-2].
5. A test that accepts either of two outcomes pins the branch instead [RISK-138-6, CR-138-2].
6. Domain refusals (kill switch, rate limit, funds, bad input) map to 4xx with their own code, not 500; "unwired" differs from "stopped" [RISK-137-1, CR-137-2].
7. The unreadable-means-engaged rule lives in the DTO/mapping code, not only in a port doc comment [RISK-137-2].
8. Text sent to the UI strips Unicode format/bidi characters; stored alarm fields have a length cap [SEC-137-1, SEC-137-2].
9. Startup reads each setting once (mode from one source); port 0 and relative or non-canonical dirs are rejected [RISK-138-4, CR-24-2, SEC-138-1].
10. Refusal text names only commands that exist; error logs in a loop are rate-limited; `live-trading` is forwarded and tests pass with it on [RISK-142-5, CR-142-4, RISK-138-3].
