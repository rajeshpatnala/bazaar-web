# 5. Reconnecting WebSocket client

(From `bz-market-data` SKILL.md; open when the task touches this area.)


One task owns the socket; everything else talks to it via channels.

```rust
// state: desired subscriptions are the source of truth, not the socket
struct Desired { modes: HashMap<StreamMode, BTreeSet<Token>> }

loop {
    match connect(&cfg).await {                       // connect timeout from config
        Ok(ws) => { backoff.reset_after(stable_for); run(ws, &mut desired, &tx).await }   // returns why it ended
        Err(e) if e.is_auth() => { tx.send(Status(SessionExpired)); break }              // do not hammer
        Err(_) => {}
    }
    tx.send(Status(Reconnecting { attempt }));
    sleep(backoff.next_with_jitter()).await;          // exponential, capped, full jitter
}
```

- Backoff: exponential with cap and jitter (base/cap in config); reset only after the connection stayed healthy for a minimum time, so a flapping server does not reset it every cycle.
- On every (re)connect, resend the whole desired set, grouped per mode and chunked to the vendor's batch size; never assume the server remembers.
- Liveness: any frame counts, including the 1-byte heartbeat. If nothing arrives for `stale_after` seconds during market hours, close and reconnect; outside hours do not alarm (use the calendar from `bz-core`).
- Backpressure: bounded channel; when the consumer lags, drop to "latest tick per instrument" (conflate) and count drops. Never let the socket reader block.
- Shut down on a cancellation token; no detached tasks.
- Parser output is `Vec<Tick>` with `Decimal` prices; the divisor comes from the instrument's segment (looked up), not guessed from the packet.
