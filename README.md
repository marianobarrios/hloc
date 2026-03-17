# hloc 

**hloc** (Historical Lines of Code) scans a directory tree for Git repositories and produces a report showing how lines of code evolved _over time_.

[![CI](https://github.com/marianobarrios/hloc/actions/workflows/ci.yml/badge.svg)](https://github.com/marianobarrios/hloc/actions/workflows/ci.yml)

## Features

- hloc uses [tokei](https://github.com/XAMPPRocky/tokei) for counting individual files, inheriting its speed and accuracy.

- hloc is a simple tool that does just counting. It does not analyze diffs, or line survival.

- hloc counts lines in-place and in-memory, without cloning, creating worktrees or touching the existing working copy in any way.

- hloc is fast: it uses aggressive parallelism and can count decades-old repositories with hundreds of thousands of lines in a few second in a modern computer.

## Installation from source

**Prerequisites:** [Rust](https://rustup.rs).

```sh
cargo install --path .
```

Or just build and run directly:

```sh
cargo run --release -- <BASE_DIR>
```

## Usage

```
hloc [OPTIONS] <BASE_DIR>...
```

| Argument / Option              | Description |
|--------------------------------|---|
| `<BASE_DIR>...`                | One or more root directories to search for Git repositories (recursive) |
| `-o, --output-dir <DIRECTORY>` | Where to write the HTML report (default: `out/`) |
| `-c, --config <CONFIG_FILE>`   | TOML configuration file (see below) |
| `-p, --period <PERIOD>`        | Time granularity for sampling commits: `auto` (default), `week`, `month`, or `quarter`. `auto` picks the finest granularity that keeps the chart under 200 data points |
| `-s, --suppress-progress`      | Do not print progress to stderr |
| `--show-resolved-config`       | Print the resolved per-repository configuration as TOML and exit |
| `--languages`                  | Print the list of supported languages with their file extensions and exit |

## Configuration file

The config file is a TOML map of [Unix glob patterns](https://en.wikipedia.org/wiki/Glob_(programming)) to per-repository settings. Multiple patterns can match the same repository; their settings are merged.

```toml
# Apply to every repository
["**/*"]
min_lines = 5000
skip_languages = ["Xml", "Json", "Yaml"]

["**/generated-sdk"]
ignore = true

# Only count history from a certain date
["**/legacy-monolith"]
from_time = 2020-01-01

# Mark as archived — the line count won't be propagated past the last commit
["**/old-service"]
archived = true
```

### Available settings

| Key | Type | Default | Description |
|---|---|---|---|
| `ignore` | bool | `false` | Exclude matching repositories from the report entirely |
| `skip_languages` | \[string\] | `[]` | Languages to exclude from the line count (uses tokei language names, e.g. `"Rust"`, `"TypeScript"`; run `hloc --languages` for the full list) |
| `min_lines` | integer | `1` | Minimum lines of code a repository must reach at any point to appear in the report |
| `from_time` | date | — | Ignore commits before this date (`YYYY-MM-DD`) |
| `archived` | bool | `false` | Cap the repository's history at its last commit instead of propagating to the current date |
| `fork_priority` | integer | `0` | Priority used during fork detection. When two repositories share commit history, the one with the lower value is treated as the original and retains those commits; the other has the shared commits removed from its count. Ties are broken alphabetically by path. |

Use `--show-resolved-config` to inspect the final merged settings for every discovered repository before running a full count.

## Works well with

[**git-workspace**](https://github.com/orf/git-workspace) is a tool that clones and keeps in sync an entire organization's repositories from GitHub, GitLab, or Gitea into a single local directory tree. That makes it a natural companion to hloc: use git-workspace to maintain a up-to-date local mirror of all your repos, then point hloc at the same directory to generate the report.

```sh
# Sync all repositories in your organisation
git-workspace update

# Generate the hloc report over the synced workspace
hloc ~/workspace --config config.toml
```
