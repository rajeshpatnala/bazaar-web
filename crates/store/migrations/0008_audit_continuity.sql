-- Defence in depth for the audit chain (SEC-123-4): the database itself refuses an insert whose
-- seq is not last+1 or whose prev_hash is not the last row's hash (32 zero bytes when empty).
-- The hash is still verified by the application (verify on open); SQL cannot compute it.
CREATE TRIGGER audit_log_continuity BEFORE INSERT ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log seq must follow the last row')
    WHERE NEW.seq IS NOT COALESCE((SELECT MAX(seq) FROM audit_log), 0) + 1;
    SELECT RAISE(ABORT, 'audit_log prev_hash must equal the last hash')
    WHERE NEW.prev_hash IS NOT COALESCE(
        (SELECT hash FROM audit_log ORDER BY seq DESC LIMIT 1), zeroblob(32));
END;
