# How to Use AI with Statespace
_A practical guide for contributing to Statespace using AI coding assistants._

Statespace benefits from thoughtful AI-assisted development, but contributors must maintain high standards for code quality, security, and collaboration. This guide will help you contriute effectively.

---

## Core Principles

- **Human Oversight**: You are accountable for all code you submit. Never commit code you don’t understand or can’t maintain.
- **Quality Standards**: AI code must meet the same standards as human written code—tests, docs, and patterns included.
- **Transparency**: Be open about significant AI usage in PRs and explain how you validated it.

---

## Best Practices

**✅ Recommended Uses**

- **Boilerplate code** - Struct definitions, trait implementations, error type variants
- **Tests** - Writing unit tests for existing functions (we need more coverage in CLI commands)
- **Documentation** - Rustdoc comments, README updates, inline explanations
- **Refactoring** - Extracting functions, renaming, reorganizing modules
- **Routine implementations** - Standard patterns like `From` conversions, builders, serialization

**❌ Avoid AI For**

- **Security-critical code** - SSRF protection, path traversal prevention, credential handling
- **Architectural changes** - New crate structure, API redesigns, protocol changes
- **Code you don't understand** - If you can't explain what it does, don't submit it
- **Complex async logic** - Race conditions and subtle concurrency bugs are hard for AI to reason about
- **Gateway API integration** - Authentication flows and API contracts require careful coordination

**Workflow Tips**  

- Start small and validate often. Build, lint, and test incrementally
- Study existing patterns before generating new code
- Always ask: "Is this secure? Does it follow project patterns? What edge cases need testing?"

**Security Considerations**  

- Never expose secrets in prompts
- Sanitize inputs/outputs and follow Statespace’s established security patterns

---

## Testing & Review

Before submitting AI-assisted code:

1. **Read and understand every line** - Can you explain what it does and why?
2. **Run the full check suite** - `cargo fmt`, `cargo clippy`, `cargo test`
3. **Verify it handles errors properly** - No `unwrap()`, `expect()`, or `panic!()`
4. **Check for security implications** - User input validation, path handling, network requests
5. **Test edge cases** - AI often generates happy-path code that fails on edge cases

**Always get human review** for:

- Security sensitive code
- Core architecture changes
- Async/concurrency logic
- Protocol implementations
- Large refactors or anything you’re unsure about

---

## Community & Collaboration

- In PRs, note significant AI use and how you validated results  
- Share prompting tips, patterns, and pitfalls  
- Be responsive to feedback and help improve this guide  

---

## Red Flags to Watch For

AI-generated Rust code often has these issues:

- Uses `unwrap()` or `expect()` (denied by our lints)
- Missing error propagation with `?`
- Overly complex solutions when simple ones exist
- Incorrect lifetime annotations
- Unsafe code (forbidden in this project)
- Hardcoded values that should be configurable
- Missing or incorrect error handling in async code

---

## Questions?

Join our [Discord][discord-link] or [GitHub Discussions][gh-discussions] to get help, find collaorators, and/or talk more about responsible AI development.



[discord-link]: https://discord.com/invite/rRyM7zkZTf
[gh-discussions]: https://github.com/orgs/statespace-tech/discussions
