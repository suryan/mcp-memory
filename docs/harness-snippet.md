# Harness snippet

Point the harness at the `mcp-memory` binary. No arguments: it serves MCP on stdio.

```json
{
  "mcpServers": {
    "memory": {
      "command": "mcp-memory"
    }
  }
}
```

The server publishes its own instructions. If a client drops them, use the policy below.

Recall before non-trivial work, and again when the task moves to a new area. Call `memory_recall` with the user request in your own words. Use the hits. If the result is empty, continue and do not invent memories.

In learn mode, call `memory_remember` with `explicit` false when the user corrects you, states a durable preference, or the session discovers a repo fact, command, convention, or dead end. Set `type` to `preference`, `fact`, `convention`, `correction`, or `ruled_out`. Set `scope` to `global` for how the user wants work done, and `repository` for something true about this repo. Attach entities and relations when you know them. Update an existing id instead of adding a near-duplicate. If new information contradicts an active memory, pass `supersedes` with the old id.

Set `explicit` true only when the user asked you to remember, change, or forget. `read_write` accepts only explicit writes. `read_only` and `off` reject agent writes. Delete by id only.

Do not remember one-off task instructions, anything already in project rules, secrets, or status that will be stale tomorrow. Project rules win when they conflict with a memory.
