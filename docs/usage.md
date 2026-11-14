# Usage

`mcp-memory` is a local MCP server. A coding harness starts the binary with no arguments and talks to it over stdio. Statements are stored in an embedded HelixDB graph on disk, one personal space per machine.

## Install

From this repo, with Rust 1.99.1 (pinned in `rust-toolchain.toml`):

```bash
cargo build --release
```

The binary is `target/release/mcp-memory`. Put that path in the harness config, or install the binary onto `PATH` and use the name `mcp-memory`.

## Connect a harness

```json
{
  "mcpServers": {
    "memory": {
      "command": "mcp-memory"
    }
  }
}
```

No arguments. The process serves MCP until the harness closes stdin. Logs go to stderr. If `HELIX_TELEMETRY_LEVEL` is unset, the process sets it to `off` before opening the graph.

The server sends its own instructions. For a client that drops them, the same policy is in [harness-snippet.md](harness-snippet.md).

## Access

Default mode is `learn`. Change it from the CLI. The human CLI can still list, show, and delete when the agent cannot write.

| Mode | Agent |
|---|---|
| `off` | No recall and no writes |
| `read_only` | Recall and list only |
| `read_write` | Recall, plus writes the user asked for (`explicit: true`) |
| `learn` | Recall, explicit writes, and automatic writes |

```bash
mcp-memory config show
mcp-memory config set read_only
```

## What to store

One statement: a title, a one-line summary, and the content. Types are `preference`, `fact`, `convention`, `correction`, and `ruled_out`. Scope is `global` (how you want work done) or `repository` (true of this repo).

Leave out chat transcripts, file bodies, one-off task instructions, anything already in project rules, and secrets. Secret-shaped text is rejected and is not written to the log. Project rules win when they disagree with a memory.

## Agent tools

- `memory_recall` — task wording in, relevant active memories out. An unrelated query comes back empty.
- `memory_remember` — create, update, or supersede one memory. Set `explicit` to true only when you asked for the write. Pass `supersedes` with the old id when a new statement replaces one.
- `memory_forget` — delete one memory by id. There is no delete-by-query.
- `memory_list` — browse what is stored. This is not task recall.
- `memory_graph` — entities linked to a name, and the memories that cite them.
- `memory://digest` — titles and summaries for global memories and the current repository.

Pass `project_path` when the harness knows the repo. Otherwise the server uses its working directory. Recall returns global memories plus that repository, and nothing from a different repo.

## Human CLI

```bash
mcp-memory health
mcp-memory list
mcp-memory list --scope global --type preference
mcp-memory show <id>
mcp-memory search "integration tests"
mcp-memory search "seed the database" --project-path /path/to/repo
mcp-memory delete <id>
mcp-memory export memories.json
mcp-memory import memories.json
```

Export omits embeddings. Import rebuilds them. Importing the same id with different content marks that memory `conflicted` and leaves it out of recall.

## Where the data is

`$MCP_MEMORY_HOME`, or `$XDG_DATA_HOME/mcp-memory`, or `~/.local/share/mcp-memory`.

`config.toml` is in that directory. The graph is in `helix/` next to it. Stopping the process does not drop the graph.

## Limits of this build

Recall uses keyword, graph links, and a local `token-hash-v1` embedding (384 dimensions). Shared wording ranks closer than unrelated text. This is not paraphrase recall. `mcp-memory health` reports the model id. There is no admin HTTP API and no shared team server in this build. Contributor setup is in [development.md](development.md).
