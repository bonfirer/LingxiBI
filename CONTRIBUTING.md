# Contributing

Thanks for your interest in improving HISENSE LingxiBI!

## Getting started

1. Fork and clone the repository.
2. Follow the local development steps in the [README](README.md#-quick-start-local-development).
3. Create a feature branch: `git checkout -b feat/my-change`.

## Development workflow

- **Backend (Rust):**
  - Build: `cd server && cargo build`
  - Lint: `cargo clippy --all-targets`
  - Format: `cargo fmt`
  - Database migrations live in `server/migrations/` and run automatically on
    startup. Add new ones as `NNN_description.sql` and register them in `main.rs`.
- **Frontend (React/TypeScript):**
  - Dev server: `cd client && npm run dev`
  - Type-check & build: `npm run build`
  - Lint: `npm run lint`
  - User-facing strings go through i18n — add keys to both
    `src/i18n/en/translation.json` and `src/i18n/zh/translation.json`.

## Pull requests

- Keep PRs focused and reasonably small.
- Make sure `cargo build` and `npm run build` both pass.
- Describe what changed and how you tested it.
- Don't commit secrets, `.env` files, build artifacts (`target/`, `dist/`), or
  `node_modules/`.

## Reporting issues

Open an issue with steps to reproduce, expected vs. actual behavior, and your
environment (OS, Rust/Node versions, database).
