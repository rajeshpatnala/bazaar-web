---
name: ci-cd
description: CI/CD for this repo - GitHub Actions checks (PR title lint, lint, tests, coverage, supply chain), caching, container builds, automatic CHANGELOG and release tags with release-please (git-cliff fallback), image publishing, environments and deploy approvals, dependency automation, and CI secrets handling. Use when creating or changing workflows, Dockerfiles, release or deployment automation.
---

# CI/CD

The kit ships working defaults; adapt them, don't reinvent them:

| File | Purpose |
|---|---|
| `.github/workflows/ci.yml` | PR + main checks: PR title lint, stack lint/tests/coverage, supply chain, image build |
| `.github/workflows/release.yml` | release-please: release PR with CHANGELOG + version bump; on merge -> tag `vX.Y.Z`, GitHub release, image push |
| `release-please-config.json`, `.release-please-manifest.json`, `version.txt` | Release config and version source |
| `cliff.toml` | git-cliff config for local/offline changelog (`just changelog`) |
| `.github/dependabot.yml` | Weekly grouped dependency updates |
| `.github/pull_request_template.md` | PR body skeleton |
| `Dockerfile`, `.dockerignore` | Container build (if the project ships one) |

## 1. Pipeline rules

- Required checks on every PR: PR title, lint, tests, supply chain, image (if any). A human enables branch protection; document required settings in the PR.
- PR titles **must** be Conventional Commits (`feat(scope): ...`): squash merges make the title the commit, and the changelog is generated from it.
- Jobs skip cleanly when the project isn't bootstrapped yet (`if: hashFiles('<manifest>') != ''`).
- `just check` locally mirrors CI.
- PR CI target under 10 minutes: fix caching before adding runners.
- Pin actions to a major version (or SHA); check current majors with web search when editing.
- Minimal `permissions:` per workflow; `concurrency` cancels superseded runs.

## 2. Releases, CHANGELOG and tags (automatic)

1. Feature PRs merge with Conventional Commit titles.
2. `release.yml` keeps one release PR open that bumps the version and updates `CHANGELOG.md`.
3. The tech lead merges the release PR after user approval -> tag `vX.Y.Z`, GitHub release, image built and pushed in the **same** workflow run (tags created with `GITHUB_TOKEN` don't trigger other workflows).
4. Never hand-edit generated changelog entries; fix PR titles instead. No GitHub: `just changelog` + annotated tag by the tech lead.

SemVer: feat -> minor, fix/perf -> patch, breaking -> major (pre-1.0: breaking bumps minor).

## 3. Images, deployment, secrets

- Images tagged `vX.Y.Z` + `sha-<short>`, immutable, promoted by digest; production deploys only through environments with required human reviewers; agents never deploy to production.
- Only GitHub encrypted secrets / OIDC; never secret values in YAML, Dockerfiles, build args or logs. List required secret **names** in the PR.

## 4. Stack specifics

- Rust: `dtolnay/rust-toolchain@stable` installs rustup; `rust-toolchain.toml` pins the actual version. Cache with `Swatinem/rust-cache`; `cargo-nextest` and `cargo-llvm-cov` via `taiki-e/install-action`; cargo-deny via `EmbarkStudios/cargo-deny-action` with `deny.toml`. The Keychain store is a macOS-only target dependency, so Linux jobs build with the env secret store.
- Rust jobs: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p bazaar-broker --features live-trading -- -D warnings` (keeps the gated crate honest; no live calls), `cargo llvm-cov nextest --no-tests=pass --fail-under-lines 75` on ubuntu and `cargo nextest` on macos (the owner's platform; compiles the Keychain store), doc tests. An `openapi-drift` step regenerates `web/openapi.json` and fails if the committed UI types are stale.
- UI job (`web/`): `pnpm/action-setup` + `actions/setup-node` (`web/.nvmrc`, pnpm cache); `pnpm install --frozen-lockfile`, `typecheck`, `lint`, `test:ci`, `build`; Playwright smoke (`pnpm exec playwright install --with-deps chromium`, `pnpm e2e`) against `dev:mock` on port 5173.
- Supply chain: cargo-deny, `pnpm audit --prod`, gitleaks on every PR, dependabot for cargo, npm and github-actions.
- Tests never use secrets or the network: no Kite keys in CI, fixtures only.
- Release: release-please bumps `crates/server/Cargo.toml` (literal `version`) and `web/package.json`. Nothing is packaged or deployed: Bazaar runs locally from the repo (`just run`).

## 5. Checklist

- [ ] Workflow parses (`actionlint` if available) and runs on a test PR.
- [ ] Minimal permissions; no secret values; required secret names listed.
- [ ] `just check` matches CI; release flow verified.
