# 7. WebSocket hub (`ws.rs`; messages in the reference)

(From `bz-web-api` SKILL.md; open when the task touches this area.)


- Upstream: the feed port delivers raw ticks into a bounded `mpsc`. One **coalescer** task keeps the latest tick per symbol and every ~250 ms (`tokio::time::interval`, `MissedTickBehavior::Skip`, period injectable for tests) publishes the batch on `Hub.ticks` (`broadcast`). Order, gate, job and status events go on a second `broadcast` (`Hub.events`) so a tick burst can never evict them.
- Each upgrade runs after `host_guard`, `origin_guard` and `session_guard`; reject over the connection cap (4) with 429 problem+json before upgrading. `max_message_size` 4 KiB. The hub also caches the last tick per symbol so a new `subscribe` gets a snapshot immediately.
- Session loop: split the socket; `select!` over shutdown token, a 20 s ping interval (close if nothing heard for 60 s), client frames, `ticks.recv()` filtered by that session's subscription set, `events.recv()`. Sends use a 5 s timeout; a slow or dead client is closed and the browser reconnects and refetches over REST.
- `RecvError::Lagged` is handled explicitly: send `{"type":"resync",...}` (UI invalidates the listed query groups) and continue; `Closed` ends the session. A gap in the coalescer clock (wall time jumped past a few seconds, i.e. the Mac slept) also emits `data_status` stale plus `resync`.
- Client frames: only `subscribe`/`unsubscribe`; malformed JSON, binary frames, more than 200 symbols or any order-like message close the socket with 1008. The socket never carries an action that changes state.
- Shutdown: `axum::serve` waits for open sockets, so sessions must watch the shutdown token and close with 1001; cap the whole drain at about 10 s.

```rust
loop {
    tokio::select! {
        _ = shutdown.cancelled() => { let _ = tx.send(close(1001)).await; break; }
        _ = ping.tick() => { if last_heard.elapsed() > STALE { break; } if send(&mut tx, Message::Ping(Bytes::new())).await.is_err() { break; } }
        frame = rx.next() => match frame { Some(Ok(m)) => { last_heard = Instant::now(); if !client_frame(m, &mut subs) { break; } } _ => break },
        t = ticks.recv() => match t {
            Ok(m) if subs.contains(&m.symbol) => if send_json(&mut tx, &m).await.is_err() { break },
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => if send_json(&mut tx, &WsOut::resync("lagged")).await.is_err() { break },
            Err(RecvError::Closed) => break,
        },
        e = events.recv() => { /* same shape; every event goes to every session */ }
    }
}
```
