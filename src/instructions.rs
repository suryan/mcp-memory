//! Server instructions sent to the harness at initialize.

/// The recall and remember policy the model sees. Part of the product, not a hint.
pub const INSTRUCTIONS: &str = "\
Memory is a local knowledge graph of durable working context. It is not a transcript and it does not replace project rules.

Recall: before non-trivial work, and again when the task moves to a new area, call memory_recall with the user request in your own words. Use what comes back. If the result is empty, proceed and do not invent memories.

Repository: pass project_path, the absolute path of the repository you are working in, to memory_recall and memory_remember. The server does not know your working directory. Without project_path, recall covers only global memories and every write is stored as global.

Remember: in learn mode, call memory_remember with explicit=false when the user corrects you, states a durable preference, or the session discovers a repo fact, command, convention, or dead end that would be expensive to rediscover. Set type to preference, fact, convention, correction, or ruled_out. Set scope to global for how the user wants work done, and repository for something true about this repo. Attach entities and relations when you know them. A relation kind is one of REQUIRES, PART_OF, RUNS_WITH, PREFERS, CORRECTS, or RULED_OUT_FOR. Update an existing id instead of adding a near-duplicate. If new information contradicts an active memory, pass supersedes with the old id.

Explicit writes: set explicit=true when the user asked you to remember, change, or forget. read_write mode accepts only explicit writes. read_only and off reject writes.

Do not remember one-off task instructions, anything already in project rules, secrets, or status that will be stale tomorrow. Do not store tokens, private keys, or .env secrets.

Project rules and steering win when they conflict with a memory.\
";
