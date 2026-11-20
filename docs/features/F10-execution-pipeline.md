# F10: Execution pipeline

Plan: `.agents/plans/F10-execution-pipeline.md`. Status: in progress.

## Slice 0 (shared contracts)
- `core::ports`: `IdGen`, `OrderRepo`, `ApprovalRepo`, `AuditRepo`, `KillSwitchRepo`, `PaperLedgerRepo`, `MarginSource`, `QuoteSource`, plain-data records, `RepoError::Conflict`.
- `config/risk.toml` `[execution]`: approval, drift, order-rate, kill-switch phrase and paper keys. All new values are UNVERIFIED, owner to confirm.
- `execution`: `IntentId`, `OrderIntent`, `OrderState::apply`/`ExecEvent`, `ExecError`, `Executor`, `ApprovedOrder` (`pub(crate)` constructor), audit hash and `verify_chain`.
- ADR: `docs/adr/0001-audit-hash-chain.md`.

## Deferred
- Rows-only chain cannot detect tail truncation (see ADR).
