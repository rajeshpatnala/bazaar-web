---
paths:
  - "**/*.ts"
  - "**/*.tsx"
---
# TypeScript and React files

- Follow `fe-standards` (load the skill if it isn't preloaded) and the area skill for what you're changing.
- No `any`, no `enum`; Zod at every API boundary; named exports; one-way imports `app -> pages -> features -> shared`.
- Design-system components and tokens only; no raw colours or spacing; accessible names and keyboard support.
- Nothing secret behind `VITE_`/`NEXT_PUBLIC_`; no `innerHTML`/`dangerouslySetInnerHTML` outside `SafeHtml`.
- After adding or moving modules, run `just codemap`.
