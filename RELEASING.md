# Releasing geocoder-core

A release is a version tag (`vX.Y.Z`) pushed on `main`. Two workflows run
on it:

| Workflow | Publishes |
|---|---|
| [`Release`](.github/workflows/release.yml) | The GitHub release (notes from `CHANGELOG.md`), then the `geocoder-core` crate on crates.io, after an approval in the `release` environment |
| [`Image`](.github/workflows/docker.yml) | `creatiwity/geocoder-api:vX.Y.Z`, `:vX.Y`, `:vX` and `:latest` on Docker Hub |

`geocoder-api` is not published on crates.io (`publish = false`): it ships
as the image and the Helm chart.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org):
`feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `chore:`, `ci:`, `build:`,
`test:`, with an optional scope (`fix(chart): …`) and `!` or a
`BREAKING CHANGE:` footer for a breaking change. `CHANGELOG.md` is
generated from them by [git-cliff](https://git-cliff.org)
([`cliff.toml`](cliff.toml)); other commits do not appear in it.

## Making a release

1. Pick the version (Semantic Versioning; before 1.0, a breaking change
   bumps the minor version) and set it in two places:
   - `Cargo.toml`: `[workspace.package] version`;
   - `chart/Chart.yaml`: `appVersion` (`vX.Y.Z`), and bump the chart
     `version`.

   A change of the index format (`index::format::VERSION`) or of a
   profile's `version()` makes every existing index unreadable: say so in
   the changelog, indexes must be rebuilt.
2. Generate the changelog section:

   ```sh
   git cliff --unreleased --tag vX.Y.Z --prepend CHANGELOG.md
   ```

   `--prepend` writes at the very top: move the section under the
   introduction. Review it, then check that the crate packages:

   ```sh
   cargo publish -p geocoder-core --dry-run --locked
   ```

3. Commit and tag:

   ```sh
   git commit -am "chore(release): vX.Y.Z"
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

4. `Release` checks that the tag, the crates, the chart and the changelog
   agree and creates the GitHub release. Approve the `release` environment
   in the workflow run to publish on crates.io. A failed publication can be
   re-run from the workflow: a version already published is skipped.

## One-time setup

### crates.io

1. Sign in at [crates.io](https://crates.io) with the account that owns
   the Creatiwity crates (the pgzest one).
2. [Account settings → API Tokens](https://crates.io/settings/tokens): a
   token with the `publish-new` and `publish-update` scopes, restricted to
   the `geocoder-core` crate once it exists.
3. Secret `CARGO_REGISTRY_TOKEN` in the `release` environment (or the
   repository).

After the first publication, add co-owners if needed:
`cargo owner --add github:Creatiwity:<team> geocoder-core`.

### Docker Hub

*Settings → Environments → New environment* `docker`, without reviewers,
with *Deployment branches and tags* limited to the branch `main` and the
tags `v*`. Its secrets: `DOCKER_USERNAME` and `DOCKER_PASSWORD` (an access
token with Read & Write on `creatiwity/geocoder-api` only, not the account
password). Only the `push` job of `Image` uses it, on `main` and on
version tags: other branches build and check without publishing.

### The `release` environment

*Settings → Environments → New environment* `release`, with **Required
reviewers** (the maintainers) and, under *Deployment branches and tags*,
only the tags `v*`.
