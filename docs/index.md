# Academic Routine Documentation Index

This directory provides operational, architectural, and protocol reference documentation for the Academic Routine system. These documents capture cross-cutting knowledge that cannot be represented within Rustdoc or Python docstrings alone.

## Documentation Index

| File | Purpose and Guidance |
|---|---|
| [`ucam-scraping.md`](ucam-scraping.md) | Read when the UCAM portal layout changes, routine/attendance extraction fails, or when capturing fresh test fixtures. |
| [`operations.md`](operations.md) | Read when managing systemd units, rotating credentials, executing releases, or troubleshooting synchronization errors. |
| [`state-model.md`](state-model.md) | Read when modifying state schema, investigating concurrency guarantees, or adjusting attendance auto-import rules. |
| [`decisions.md`](decisions.md) | Read to review historical and active architectural decision records before altering established patterns. |

## Documentation Placement Rules

When documenting new features or fixes, route knowledge to the appropriate destination:

- **New Portal / DOM Knowledge:** Document selectors, postback rules, and SSRS layouts in [`ucam-scraping.md`](ucam-scraping.md).
- **New Operational Steps:** Document commands, systemd parameters, and troubleshooting in [`operations.md`](operations.md).
- **New State Invariants:** Document data structures, schema migrations, and concurrency in [`state-model.md`](state-model.md).
- **Design Choices & Trade-offs:** Document rationale and rejected alternatives in [`decisions.md`](decisions.md).
- **User-Facing Behavior:** Update quickstart, command summaries, and environment variables in the root [`README.md`](../README.md).

## Authoring Conventions

1. **One Home Per Fact:** Maintain a single source of truth per specification. Cross-reference existing documents rather than duplicating text.
2. **Contract-Driven Descriptions:** Document stable interface IDs, file paths, and control behaviors rather than line numbers or internal helper names that drift across refactors.
3. **Strict Placeholder Policy:** Never commit live API tokens, credentials, student IDs, teacher phone numbers, or private Gist IDs. Use standard placeholders (`<GIST_ID>`, `CS 0000000`, `01000000000`, `example.test`).
4. **Portal Regression Workflow:** When resolving a portal layout change, update `ucam-scraping.md`, its selector reference table, and the volatility stamp within the same pull request or commit.
5. **One README File:** The repository maintains exactly one README file (`README.md` at root). Do not create sub-directory README files.

*Last reviewed against v0.3.2 (2026-10-29)*
