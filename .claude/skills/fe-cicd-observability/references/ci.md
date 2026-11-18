# CI workflow and repository settings

Build-once model, the full `ci.yml` and the repository ruleset.

## Contents

- Build once, promote everywhere
- The CI workflow
- Repository settings

## Build once, promote everywhere

*Diagram: build once, promote · one artifact from `main` (after promotion from `develop`) to production*

The artifact that passed every test on staging is exactly the one users get. Environments differ only in the `config.json` they serve, and because old releases stay deployed, rollback is a pointer change, not a rebuild.

## The CI workflow

One workflow runs on every PR and again in the merge queue, so each check in `fe-repo-foundation`, `fe-app-architecture`, `fe-auth-realtime`, `fe-design-system`, `fe-testing`, `fe-performance` becomes automatic. Fast checks run first, in parallel, so a type error fails in about two minutes instead of after a 15-minute E2E run.

| Job | Runs on | What it does | Budget |
| --- | --- | --- | --- |
| `pr-title` | PR | PR title follows Conventional Commits (it becomes the squash commit) | < 30 s |
| `verify` | PR + queue | Types, lint, CSS lint, format, API contract, unit/component tests, build, size budget, Lighthouse CI | 10 min |
| `visual` | PR + queue | Storybook story tests and visual regression in Chromium | 5 min |
| `preview` | PR + queue | Deploys the PR build to a unique URL for reviewers and E2E | 3 min |
| `e2e-smoke` | PR | `@smoke` Playwright tests against the preview | 5 min |
| `e2e-full` | Merge queue | Full Playwright suite, sharded across 4 runners | 15 min |

**`.github/workflows/ci.yml`**

```yaml
name: ci
on:
  pull_request:
  merge_group:

permissions:
  contents: read # least privilege by default; jobs opt in to more

concurrency:
  group: ci-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: true # a new push cancels the outdated run

jobs:
  pr-title:
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    permissions:
      pull-requests: read
    steps:
      - uses: amannn/action-semantic-pull-request@v5 # pin to a commit SHA in real repos
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}

  verify:
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v5
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v5
        with:
          node-version-file: .nvmrc
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: pnpm typecheck
      - run: pnpm lint
      - run: pnpm lint:css
      - run: pnpm format:check
      - run: pnpm api:check
      - run: pnpm test:unit --run --coverage
      - run: pnpm build
        env:
          VITE_RELEASE: ${{ github.sha }}
      - run: pnpm size
      - run: pnpm dlx @lhci/cli autorun
      - uses: actions/upload-artifact@v4
        if: ${{ !cancelled() }}
        with:
          name: verify-reports
          path: |
            coverage
            .lighthouseci

  visual:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v5
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v5
        with:
          node-version-file: .nvmrc
          cache: pnpm
      - run: pnpm install --frozen-lockfile
      - run: pnpm exec playwright install --with-deps chromium
      - run: pnpm vitest run --project storybook --project vrt
      - uses: actions/upload-artifact@v4
        if: failure()
        with:
          name: visual-diffs # reference, actual and diff images for reviewers
          path: '**/.vitest/attachments/**'

  preview:
    needs: verify # runs for PRs and for merge-queue builds
    runs-on: ubuntu-latest
    environment: preview
    permissions:
      contents: read
      id-token: write # OIDC: short-lived cloud credentials, no stored keys
    outputs:
      url: ${{ steps.deploy.outputs.url }}
    steps:
      - uses: actions/checkout@v5
      # … pnpm, node, install, build (same as verify)
      - id: deploy
        run: ./scripts/deploy-preview.sh "${{ github.event.pull_request.number || github.sha }}" # prints url=… to $GITHUB_OUTPUT

  e2e-smoke:
    if: github.event_name == 'pull_request'
    needs: preview
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      # … pnpm, node, install, playwright install --with-deps
      - run: pnpm exec playwright test --grep @smoke
        env:
          E2E_BASE_URL: ${{ needs.preview.outputs.url }}
          E2E_SEED_TOKEN: ${{ secrets.E2E_SEED_TOKEN }}

  e2e-full:
    if: github.event_name == 'merge_group'
    needs: preview
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        shard: [1, 2, 3, 4]
    steps:
      - uses: actions/checkout@v5
      # … pnpm, node, install, playwright install --with-deps
      - run: pnpm exec playwright test --shard=${{ matrix.shard }}/4
        env:
          E2E_BASE_URL: ${{ needs.preview.outputs.url }}
          E2E_SEED_TOKEN: ${{ secrets.E2E_SEED_TOKEN }}
      # blob reports and merge-reports: see `fe-testing`
```

Add `"test:unit": "vitest --project unit"` and `"test:visual": "vitest run --project vrt"` to `package.json` if not already present (`fe-testing`). Locally, `pnpm verify` still runs the fast subset in the same order.

## Repository settings

The workflow only protects your branches if the repository requires it. Configure these once with a ruleset on **both** `develop` (the integration branch) and `main` (the release branch) (Settings → Rules) and treat changes to them like code changes: reviewed, and recorded in an ADR.

| Setting | Value | Why |
| --- | --- | --- |
| Require a pull request | On; 1 approval (2 for paths marked high-risk in CODEOWNERS) | No unreviewed code on `develop` or `main`; promotions to `main` need a human approval |
| Require review from Code Owners | On | The owning team approves changes to its area |
| Dismiss stale approvals on new commits | On | Approval covers the code that actually merges |
| Require conversation resolution | On | No review comment silently ignored |
| Required status checks | `pr-title`, `verify`, `visual`, `e2e-smoke`, and `e2e-full` (queue) | The gates from the previous section |
| Require merge queue | On; squash merge; build concurrency ≥ 3 | Tests each change on top of the ones ahead of it |
| Require linear history | On | Readable history; each commit is one PR |
| Block force pushes and deletion | On | `develop` and `main` can't be deleted or rewritten |
| Allowed merge method | Squash only | One commit per PR, titled in Conventional Commits |
| Bypass list | Empty (emergency access via a break-glass role that is audited) | Rules apply to everyone, including admins |

**Repository security settings**

- Secret scanning and push protection on, so a leaked key is blocked before it reaches the remote.
- Dependabot or Renovate security alerts on (Renovate config in `fe-repo-foundation`).
- Default `GITHUB_TOKEN` permission set to read-only; workflows request more per job.
- Environments (`preview`, `staging`, `production`) hold their own secrets. `staging` deploys from `develop`; `production` requires the deploy to come from `main` (reached only by promotion), plus an approval or wait timer.
- Cloud deploys authenticate with OIDC (`id-token: write`), so no long-lived cloud keys are stored in GitHub.
