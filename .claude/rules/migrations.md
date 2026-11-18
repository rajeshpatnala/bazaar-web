---
paths:
  - "**/migrations/**"
---
# Migrations

- sqlx migrations, forward-only and reviewed; the audit-log table is append-only (no UPDATE/DELETE paths).
- Destructive statements need an explicit note for the tech lead and security reviewer. Agents never apply migrations to the owner's real `data/` database.
