# AutoRename-PDF

Read @AGENTS.md first: it is the source of truth (architecture, commands, hard rules, where specs/playbooks/known.md/followup.md live). This file adds only Claude Code specifics.

## Claude Code

- `.claude/` is committed: skills, agents, rules and shared `settings.json`. `settings.local.json` is personal (gitignored).
- Skills: `/rename-pdfs <path>` (dry-run → review → apply; installed `autorename-pdf` or `cargo run`), `/build` (release packages), `/test` (gates, live tests, VM pointer).
- Agent: `pdf-debugger` for a PDF that gets a wrong name.
- Rules: `.claude/rules/gui.md` loads for `crates/app/src/gui/**`.
- `.claude/rules/*.md` and `settings.json` change only on the user's request, never on another agent's.
- Windows testing: the global `windows-vm` skill and `playbooks/windows-vm-validation.md`.
- Two-step renames for agents: `rename --plan plan.json` → show the user → `apply plan.json`.
