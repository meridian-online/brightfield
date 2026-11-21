---
description: >-
  Use when asked to release, ship, cut or tag a Brightfield version, to bump Brightfield's version or roll CHANGELOG.md for a release, or to finish a release whose tag has been pushed: its GitHub release, its Homebrew cask, or its entry on meridian.online.
argument-hint: "[X.Y.Z | patch]"
arguments: version
allowed-tools: Bash, Read, Edit, Write
---

# /release

A Brightfield release is a pushed `vX.Y.Z` tag. `.github/workflows/release.yml` does the publishing; the comments at its head say what each job proves. Its jobs, in order:

| Job | What it does |
|---|---|
| `tag-guard` | `scripts/check-release-tag.py` refuses a tag whose number differs from the version in `crates/brightfield-shell/Cargo.toml` or that has no `CHANGELOG.md` section, before anything is built. |
| `build` | Packages, signs and notarises a `.tar.gz` and a `.dmg` for `aarch64-apple-darwin` and `x86_64-apple-darwin`, each with a `.sha256` sidecar. |
| `verify` | Runs the packaged binary with the network denied. It reports on its own and does not hold up the jobs after it. |
| `release` | Publishes the GitHub release, with the tag's changelog section at the head of its notes. |
| `update-homebrew` | Writes the cask into `meridian-online/homebrew-tap`. It refuses a build that was not notarised. |
| `brew-install` | Installs the published cask on a clean hosted Mac and runs the installed app. |

A release has three stages, and the session's part is either side of the tag:

1. The release pull request: the version bump and the rolled changelog.
2. Hugh merges it and pushes the tag. Then the session watches the release run and checks what it published.
3. The meridian.online pull request: the changelog entry and the release row the site reads.

**You never push to `main`, never create, move or delete a tag, and never merge a pull request.** Stage 1 ends when its pull request is ready: say so with its link, and stop. Stages 2 and 3 start when Hugh says the tag is pushed, in the same session or a later one. A release is finished when the web pull request is ready, not when the tag's run goes green: web's full check for a release with no entry runs weekly, so nothing fails at merge time when stage 3 is skipped.

## Choosing the version

Use the version the argument gives. Without one, take the next patch (`0.1.9` → `0.1.10`). Every release so far has been a patch on `0.1`, and a minor version or 1.0 is Hugh's to name.

## 1. The release pull request

**Check there is a release to cut, and that the last one is finished:**

Run this block whole, from Brightfield's checkout or any worktree of it. Shell variables do not outlive one Bash call, so each block below that needs `WEB` or `PREV` sets it again.

```bash
WEB="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/../web"
git -C "$WEB" remote get-url origin | grep -q 'meridian-online/web' || { echo "no web checkout at $WEB" >&2; exit 1; }
git fetch origin && git -C "$WEB" fetch origin || exit 1
PREV=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' origin/main)
echo "previous release: $PREV"; echo "merged since:"
git log "$PREV"..origin/main --oneline --no-merges
git -C "$WEB" show origin/main:src/data/releases.json | grep -q "\"brightfield\", \"version\": \"${PREV#v}\"" \
  && echo "web lists $PREV" || echo "web has no row for $PREV"
```

`WEB` is the web checkout beside Brightfield's main checkout; the block stops when it is not there. When nothing merged since the previous release, there is nothing to release: say so and stop. When web has no row for the previous release, do stage 3 for it first, then come back.

Then work in a worktree of an up-to-date `main`, beside the other worktrees of Brightfield's main checkout rather than inside the one the session started in:

```bash
MAIN="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
git worktree add -b release-X.Y.Z "$MAIN/.claude/worktrees/release-X.Y.Z" origin/main && cd "$MAIN/.claude/worktrees/release-X.Y.Z"
```

**Reconcile the changelog.** Each pull request with a change a user would notice adds its entry under `## [Unreleased]` as it lands. Read the log above against `[Unreleased]` and add any entry that is missing.

Write each entry in the voice of the existing ones: what a person using the app notices, under `### Added`, `### Changed` or `### Fixed`, with no commit mechanics. Add an entry for anything the release itself ships for the first time, such as a new way of signing or a new install channel, since no pull request adds that line. The section becomes the GitHub release's notes, and this repository is public, so it carries no private planning identifiers.

**Roll the changelog.** Rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` with today's date (`date +%F`), and put a fresh, empty `## [Unreleased]` above it. The file has no link footer to update. Leave earlier releases' sections as written: each is that release's record, advice included.

**Bump the version** in `crates/brightfield-shell/Cargo.toml`, the one crate version a release changes, then have Cargo rewrite its line in `Cargo.lock`:

```bash
cargo metadata --format-version 1 > /dev/null
git diff --stat      # CHANGELOG.md, the shell crate's Cargo.toml, and one line of Cargo.lock
```

Other copies of the old version string in the tree, such as cask self-test fixtures and recorded measurements of that build, describe that build. Leave them.

**Check the tree accepts the tag:**

```bash
PREV=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' origin/main)
python3 scripts/check-release-tag.py --tag vX.Y.Z     # exit 0: "... agree; release notes are that section."
python3 scripts/check-release-tag.py --tag "$PREV"    # exit 4: the old tag no longer names this tree
cargo xtask ci --step release-tag-rehearsal
cargo xtask ci --step release-tag-wiring
cargo xtask ci --step fmt
cargo xtask ci --step clippy
```

Commit the three files by name, run `cargo xtask ci --step comment-citations`, push, and open the pull request:

```bash
git add CHANGELOG.md crates/brightfield-shell/Cargo.toml Cargo.lock
git commit -m "Version Brightfield X.Y.Z"
git push -u origin release-X.Y.Z
gh pr create --title "Version Brightfield X.Y.Z" --body "<what the section adds, how it was checked, and: merging this and pushing vX.Y.Z publishes the release>"
```

Then take it to ready as `CLAUDE.md`'s "The gate" says for any pull request: `cargo xtask gate <pr>`, the closed roborev branch review, and the Forge link. Tell Hugh it is ready, with its link, and stop. What he runs next is his, not yours:

```bash
git switch main && git pull --ff-only
git tag -a vX.Y.Z -m "Brightfield X.Y.Z" && git push origin vX.Y.Z
```

## 2. Watch the release run

Once Hugh says the tag is pushed:

```bash
gh run list --workflow release.yml --limit 3
gh run watch <run-id> --exit-status
```

v0.1.9's run took 35 minutes. When it is green, check what it published:

```bash
gh release view vX.Y.Z --json assets,isPrerelease,publishedAt \
  --jq '{assets: [.assets[].name], isPrerelease, publishedAt}'
gh api repos/meridian-online/homebrew-tap/contents/Casks/brightfield.rb --jq .content \
  | base64 -d | grep -m1 -E '^\s*version'
```

The release has eight assets: the `.tar.gz` and the `.dmg` for each of the two targets, each with its `.sha256`. The cask reads `version "X.Y.Z"`. Keep `publishedAt` for stage 3.

When `isPrerelease` is `true` for a build `update-homebrew` accepted, tell Hugh: promoting it to the latest full release (`gh release edit vX.Y.Z --prerelease=false --latest`) is his step.

When a job fails, report it to Hugh with `gh run view <run-id> --log-failed`, and say which jobs had already run: a failure after `release` leaves the GitHub release published. Re-running the workflow, moving or deleting the tag and editing the tap by hand are his calls.

## 3. The meridian.online pull request

Start from Brightfield's checkout or a worktree of it, and make a worktree of web on a new branch. The block stops before it makes anything when web's checkout is not where it looks:

```bash
WEB="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/../web"
cd "$WEB" && git remote get-url origin | grep -q 'meridian-online/web' || { echo "no web checkout at $WEB" >&2; exit 1; }
git fetch origin && git worktree add -b brightfield-vX.Y.Z .claude/worktrees/brightfield-vX.Y.Z origin/main || exit 1
cd .claude/worktrees/brightfield-vX.Y.Z && npm ci
```

1. **`src/data/releases.json`**: add `{ "package": "brightfield", "version": "X.Y.Z", "date": "YYYY-MM-DD" }` after the last `brightfield` row. The date is the first ten characters of `publishedAt`, the UTC day, which web's release-coverage gate checks. It is not the changelog heading's date: a tag pushed in the morning in a timezone ahead of UTC is published on the previous UTC day, as v0.1.9 was (heading 2026-11-21, published 2026-11-20T23:16:51Z).
2. **`"signedDmg": true` on that row** when `update-homebrew` passed, which it does only for a notarised build. The home page offers its signed Download button only while the newest Brightfield row carries it, so leaving it off a signed release takes the button down. The gate checks that both disk images are on the release. Say in the pull request's description that merging it puts the Download button on the live home page.
3. **`content/changelog/brightfield-vX-Y-Z.mdx`**, in the shape of the previous Brightfield entries: frontmatter `title` (`Brightfield vX.Y.Z — <what it brings>`), `date` (the row's date, quoted), `package: brightfield` and `version` (quoted); a paragraph led by **What to try.**, one led by **Also changed.**, and the line `[vX.Y.Z release notes on GitHub →](https://github.com/meridian-online/brightfield/releases/tag/vX.Y.Z)`. Write it for a reader of the site from the changelog section, rather than pasting the section. The previous entry closes on what the build still is ("Still an early preview, macOS only, unsigned."): change what this release makes untrue. Leave out a feature whose gate in `lib/release-gates.json` is `false`, since the site keeps it unannounced: Open in Brightfield links wait on `openInBrightfield`.
4. **`content/docs/brightfield/`**: change only what this release makes untrue, such as an install channel, what Homebrew puts on disk, or the platforms built.

Check, commit and open the pull request:

```bash
set -e
WEB="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")/../web"
cd "$WEB/.claude/worktrees/brightfield-vX.Y.Z"
npm test
GH_TOKEN=$(gh auth token) node scripts/check-release-coverage.mjs --gate --base origin/main
git add src/data/releases.json content/changelog/brightfield-vX-Y-Z.mdx   # and any docs page changed
git commit -m "Brightfield vX.Y.Z's changelog entry and release row"
git push -u origin brightfield-vX.Y.Z
gh pr create --title "Brightfield vX.Y.Z's changelog entry" --body "<what the entry says, the row's date and why, and whether merging it puts up the Download button>"
```

Then close its roborev branch review (`roborev review --branch --wait`), wait for `gh pr checks <pr> --watch` to finish, link it in Forge as for the release pull request, and tell Hugh it is ready, with its link. web deploys from `main`, so Hugh's merge publishes it.

## Report

```
Released Brightfield vX.Y.Z.

  Release PR:  <link> (merged by Hugh)
  Tag:         vX.Y.Z (pushed by Hugh)
  Workflow:    <run link>, <conclusion>; verify <conclusion>
  Release:     <link>, <n> assets, <pre-release | latest>
  Homebrew:    cask at X.Y.Z, brew-install <conclusion>
  Web PR:      <link>, checks <state>, ready for Hugh
```

A line that cannot be filled yet says what it waits on.
